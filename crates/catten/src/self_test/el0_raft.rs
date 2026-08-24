//! Self-test: durable recovery of the EL0 Raft service.
//!
//! [`test_persistent_raft`] (used by the NVMe test) spawns a single node whose manifest requests
//!   durable storage, waits for it to reach a leader term, stops it, respawns it with the same
//!   manifest, and asserts the restarted node recovers and *advances* its durable term. Outcome:
//!   term/vote state survives a process restart on the NVMe-backed object store, proving the
//!   storage stack is durable.
//!
//! Cluster formation is intentionally not tested here. Operational CharlotteOS
//! membership belongs to the DNS-owned Raft node and is verified by the
//! multi-guest deployment. This isolated, network-disabled process is only a
//! storage-recovery fixture; it is not a CharlotteOS cluster member.
//!
//! Expected outcome: the verifier logs `SUCCESS` and calls
//! [`crate::self_test::results::pass`] for `TestId::RaftStorage`.
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
    const ELECTION_KEY: u64 = charlotte_launch::manifest_key(b"elect-ms");
    const CLUSTER_KEY: u64 = charlotte_launch::manifest_key(b"cluster");
    const STORAGE_KEY: u64 = charlotte_launch::manifest_key(b"storage");
    const STORAGE_REQUIRED: u64 = 2;

    fn spawn_raft_node(
        manifest: &[ManifestEntry<'_>],
        ns_handle: &NameServiceHandle,
    ) -> ServiceDomain {
        let addr = crate::service::loader::load_domain(
            crate::service::store::service_elf(b"raft").expect("[el0_raft] raft.elf"),
        );
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
            address_space: addr.address_space,
            tid,
            generation,
            config_frame: addr.config_frame,
            status_frame: addr.status_frame,
        }
    }

    fn stop_domain(domain: ServiceDomain) {
        {
            let scheduler = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER.read();
            let _ = scheduler.abort_thread_generation(domain.tid, domain.generation);
        }
        supervisor::wait_domain_exit(&domain, 30_000);
        supervisor::teardown_domain(domain);
    }

    fn wait_for_single_leader(domain: &ServiceDomain, label: &str) -> u32 {
        use crate::cpu::scheduler::yield_lp;

        let status: *const u8 = {
            let base: *mut u8 = domain.status_frame.into();
            base
        };
        let deadline = crate::self_test::results::Deadline::after_millis(30_000);
        let mut next_report = 0u64;
        loop {
            let stage = unsafe {
                crate::self_test::status_u32(status, charlotte_launch::raft_status::STAGE)
            };
            let state = unsafe {
                crate::self_test::status_u32(status, charlotte_launch::raft_status::STATE)
            };
            let term = unsafe {
                crate::self_test::status_u32(status, charlotte_launch::raft_status::CURRENT_TERM)
            };
            let durable = unsafe {
                crate::self_test::status_u32(status, charlotte_launch::raft_status::DURABLE)
            };
            if stage >= 6 && state == 3 && term > 0 && durable == 1 {
                crate::logln!("[raft-storage] {} reached term {}.", label, term);
                return term;
            }
            let now = crate::cpu::scheduler::monotonic_millis();
            if now >= next_report {
                crate::logln!(
                    "[raft-storage] {} waiting: stage={}, state={}, term={}, durable={}",
                    label,
                    stage,
                    state,
                    term,
                    durable
                );
                next_report = now.saturating_add(1_000);
            }
            deadline.assert_pending(label);
            yield_lp();
        }
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

pub fn test_persistent_raft(name_service: &crate::service::supervisor::NameServiceHandle) {
    inner::test_persistent_raft(name_service);
}
