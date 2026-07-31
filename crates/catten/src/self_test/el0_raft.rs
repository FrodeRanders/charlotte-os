#[cfg(target_arch = "aarch64")]
mod inner {
    use crate::{
        ipc::ConnectionRights,
        service::{
            bootstrap::{
                ManifestEntry,
                ManifestValue,
            },
            supervisor::{
                self,
                NameServiceHandle,
                ServiceDomain,
            },
        },
    };

    const NODE_ID_KEY: u64 = charlotte_launch::manifest_key(b"node-id");
    const PEER_ID_KEY: u64 = charlotte_launch::manifest_key(b"peer-id");
    const ELECTION_KEY: u64 = charlotte_launch::manifest_key(b"elect-ms");
    const CLUSTER_KEY: u64 = charlotte_launch::manifest_key(b"cluster");
    const STORAGE_KEY: u64 = charlotte_launch::manifest_key(b"storage");
    const STORAGE_REQUIRED: u64 = 2;

    const RAFT_ELF: &[u8] =
        include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/raft.elf"));

    static mut RAFT_NS: Option<NameServiceHandle> = None;

    fn spawn_raft_node(
        manifest: &[ManifestEntry<'_>],
        ns_handle: &NameServiceHandle,
    ) -> ServiceDomain {
        let addr = crate::service::loader::load_domain(RAFT_ELF);
        let conn = crate::ipc::connection_delegate(
            ns_handle.domain.asid,
            ns_handle.endpoint_cap,
            addr.asid,
            ConnectionRights::CALL,
        )
        .expect("raft conn delegate");
        crate::service::bootstrap::write_bootstrap_cap(addr.config_frame, conn);
        crate::service::bootstrap::write_manifest(addr.config_frame, manifest);
        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(addr.entry_vaddr) };
        let tid = crate::cpu::scheduler::spawn_thread(addr.asid, entry);
        let generation = crate::cpu::scheduler::threads::MASTER_THREAD_TABLE
            .read()
            .get(tid)
            .expect("raft thread missing after spawn")
            .generation;
        ServiceDomain {
            asid: addr.asid,
            tid,
            generation,
            config_frame: addr.config_frame,
            status_frame: addr.status_frame,
        }
    }

    fn stop_domain(domain: ServiceDomain) {
        {
            let scheduler = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER.read();
            let _ = scheduler.abort_thread(domain.tid);
        }
        supervisor::wait_domain_exit(&domain, 30_000);
        supervisor::teardown_domain(domain);
    }

    fn stop_domains(first: ServiceDomain, second: ServiceDomain) {
        stop_domain(first);
        stop_domain(second);
    }

    fn wait_for_election(first: &ServiceDomain, second: &ServiceDomain, label: &str) -> (u32, u32) {
        use crate::cpu::scheduler::yield_lp;

        let first_status: *const u32 = {
            let base: *mut u8 = first.status_frame.into();
            base as *const u32
        };
        let second_status: *const u32 = {
            let base: *mut u8 = second.status_frame.into();
            base as *const u32
        };
        let mut polls = 0u64;
        let deadline = crate::self_test::results::Deadline::after_millis(30_000);
        loop {
            let first_state = unsafe { core::ptr::read_volatile(first_status.add(2)) };
            let second_state = unsafe { core::ptr::read_volatile(second_status.add(2)) };
            if (first_state == 3 && second_state == 1) || (first_state == 1 && second_state == 3) {
                let first_completions = unsafe { core::ptr::read_volatile(first_status.add(4)) };
                let second_completions = unsafe { core::ptr::read_volatile(second_status.add(4)) };
                if first_completions + second_completions > 0 {
                    let first_term = unsafe { core::ptr::read_volatile(first_status.add(5)) };
                    let second_term = unsafe { core::ptr::read_volatile(second_status.add(5)) };
                    crate::logln!(
                        "[raft] {} elected one leader (states {}/{}, terms {}/{}).",
                        label,
                        first_state,
                        second_state,
                        first_term,
                        second_term
                    );
                    return (first_term, second_term);
                }
            }
            polls += 1;
            if polls.is_multiple_of(100) {
                let first_stage = unsafe { core::ptr::read_volatile(first_status) };
                let second_stage = unsafe { core::ptr::read_volatile(second_status) };
                crate::logln!(
                    "[raft] {} waiting: stages {}/{}, states {}/{}",
                    label,
                    first_stage,
                    second_stage,
                    first_state,
                    second_state
                );
            }
            deadline.assert_pending(label);
            // Yield rather than blocking on a timer: a timer wake that is not
            // delivered would leave the deadline unchecked (silent hang).
            yield_lp();
        }
    }

    fn wait_for_single_leader(domain: &ServiceDomain, label: &str) -> u32 {
        use crate::cpu::scheduler::yield_lp;

        let status: *const u32 = {
            let base: *mut u8 = domain.status_frame.into();
            base as *const u32
        };
        let deadline = crate::self_test::results::Deadline::after_millis(30_000);
        let mut poll = 0u64;
        loop {
            let stage = unsafe { core::ptr::read_volatile(status) };
            let state = unsafe { core::ptr::read_volatile(status.add(2)) };
            let term = unsafe { core::ptr::read_volatile(status.add(5)) };
            let durable = unsafe { core::ptr::read_volatile(status.add(6)) };
            if stage >= 6 && state == 3 && term > 0 && durable == 1 {
                crate::logln!("[raft-storage] {} reached term {}.", label, term);
                return term;
            }
            if poll.is_multiple_of(100) {
                crate::logln!(
                    "[raft-storage] {} waiting: stage={}, state={}, term={}, durable={}",
                    label,
                    stage,
                    state,
                    term,
                    durable
                );
            }
            deadline.assert_pending(label);
            poll += 1;
            yield_lp();
        }
    }

    pub(super) fn test_el0_raft() {
        crate::logln!("[raft] two-node boot test");

        let ns = supervisor::node_name_service();
        crate::logln!("[raft] node name service ok asid={} tid={}", ns.domain.asid, ns.domain.tid);

        unsafe {
            RAFT_NS = Some(ns);
        }
        let _verifier = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Raft,
            verify_raft_cluster,
        );
        crate::logln!("[raft] verifier deferred");
    }

    extern "C" fn verify_raft_cluster() {
        let ns = unsafe { RAFT_NS }.expect("[raft] verifier name service missing");
        // Spawn both peers without imposing a registration order. Blocking
        // reply waits yield cooperatively, while each missing peer has one
        // deferred lookup retained by the name service.
        let r1_manifest = [
            ManifestEntry {
                key: NODE_ID_KEY,
                flags: 0,
                value: ManifestValue::Bytes(b"r1"),
            },
            ManifestEntry {
                key: PEER_ID_KEY,
                flags: 0,
                value: ManifestValue::Bytes(b"r2"),
            },
            ManifestEntry {
                key: ELECTION_KEY,
                flags: 0,
                value: ManifestValue::Unsigned(150),
            },
        ];
        let r1_domain = spawn_raft_node(&r1_manifest, &ns);
        let r2_manifest = [
            ManifestEntry {
                key: NODE_ID_KEY,
                flags: 0,
                value: ManifestValue::Bytes(b"r2"),
            },
            ManifestEntry {
                key: PEER_ID_KEY,
                flags: 0,
                value: ManifestValue::Bytes(b"r1"),
            },
            ManifestEntry {
                key: ELECTION_KEY,
                flags: 0,
                value: ManifestValue::Unsigned(150),
            },
        ];
        let r2_domain = spawn_raft_node(&r2_manifest, &ns);
        crate::logln!("[raft] nodes spawned without registration ordering");

        let r1_config = r1_domain.status_frame;
        let r2_config = r2_domain.status_frame;
        let r1: *const u32 = {
            let base: *mut u8 = r1_config.into();
            base as *const u32
        };
        let r2: *const u32 = {
            let base: *mut u8 = r2_config.into();
            base as *const u32
        };

        crate::logln!(
            "[raft] verifier running: stages {}/{}",
            unsafe { core::ptr::read_volatile(r1) },
            unsafe { core::ptr::read_volatile(r2) }
        );

        let _ = (r1, r2);
        wait_for_election(&r1_domain, &r2_domain, "local two-node cluster");
        crate::logln!("[raft] SUCCESS: local two-node service elected one leader.");
        crate::self_test::results::pass(crate::self_test::results::TestId::Raft);
        // Keep the shared node name service alive; only the Raft domains are
        // test fixtures.
        stop_domains(r1_domain, r2_domain);
    }

    pub(super) fn test_persistent_raft(ns: &NameServiceHandle) {
        crate::logln!("[raft-storage] testing NVMe-backed Raft recovery...");
        let manifest = [
            ManifestEntry {
                key: NODE_ID_KEY,
                flags: 0,
                value: ManifestValue::Bytes(b"persist"),
            },
            ManifestEntry {
                key: ELECTION_KEY,
                flags: 0,
                value: ManifestValue::Unsigned(150),
            },
            ManifestEntry {
                key: CLUSTER_KEY,
                flags: 0,
                value: ManifestValue::Bytes(b"nvmetest"),
            },
            ManifestEntry {
                key: STORAGE_KEY,
                flags: 0,
                value: ManifestValue::Unsigned(STORAGE_REQUIRED),
            },
        ];

        let first = spawn_raft_node(&manifest, ns);
        let first_term = wait_for_single_leader(&first, "first process");
        stop_domain(first);

        let restarted = spawn_raft_node(&manifest, ns);
        let restarted_term = wait_for_single_leader(&restarted, "restarted process");
        assert!(
            restarted_term > first_term,
            "restarted Raft node did not recover and advance its durable term"
        );
        stop_domain(restarted);
        crate::logln!(
            "[raft-storage] SUCCESS: term/vote state survived Raft process restart on NVMe."
        );
        crate::self_test::results::pass(crate::self_test::results::TestId::RaftStorage);
    }
}

#[cfg(target_arch = "aarch64")]
pub fn test_el0_raft() {
    inner::test_el0_raft();
}

#[cfg(target_arch = "aarch64")]
pub fn test_persistent_raft(name_service: &crate::service::supervisor::NameServiceHandle) {
    inner::test_persistent_raft(name_service);
}

#[cfg(not(target_arch = "aarch64"))]
pub fn test_el0_raft() {}
