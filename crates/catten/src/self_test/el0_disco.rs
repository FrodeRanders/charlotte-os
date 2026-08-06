//! Self-test: cluster discovery service.
//!
//! Spawns the disco service with a launch manifest and verifies it starts
//! successfully. When `disco_net_test` is active, the verifier also waits for
//! cross-node peer discovery to confirm that the Ethernet broadcast bootstrap
//! protocol works end-to-end.
#![cfg(target_arch = "aarch64")]

mod inner {
    use crate::{
        ipc::ConnectionRights,
        logln,
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
    const CLUSTER_KEY: u64 = charlotte_launch::manifest_key(b"cluster");

    static mut DISCO_NS: Option<NameServiceHandle> = None;

    fn spawn_disco(manifest: &[ManifestEntry<'_>], ns_handle: &NameServiceHandle) -> ServiceDomain {
        let addr = crate::service::loader::load_domain(
            crate::service::store::service_elf(b"disco").expect("[el0_disco] disco.elf"),
        );
        let conn = crate::ipc::connection_delegate(
            ns_handle.domain.asid,
            ns_handle.endpoint_cap,
            addr.asid,
            ConnectionRights::CALL,
        )
        .expect("disco conn delegate");
        crate::service::bootstrap::write_bootstrap_cap(addr.config_frame, conn);
        crate::service::bootstrap::write_manifest(addr.config_frame, manifest);
        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(addr.entry_vaddr) };
        let tid = crate::cpu::scheduler::spawn_thread(addr.asid, entry);
        let generation = crate::cpu::scheduler::threads::MASTER_THREAD_TABLE
            .read()
            .get(tid)
            .expect("disco thread missing after spawn")
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

    pub fn test_el0_disco() {
        logln!("Testing EL0 cluster discovery service...");

        let name_service = supervisor::node_name_service();
        let ns_asid = name_service.domain.asid;
        logln!("[disco] using node name service (asid={})", ns_asid);

        unsafe { DISCO_NS = Some(name_service) };

        let _vtid = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Disco,
            verify_el0_disco,
        );
        logln!("[disco] verifier deferred (waits for disco service + optional cross-node peer)");
    }

    extern "C" fn verify_el0_disco() {
        use crate::cpu::scheduler::yield_lp;

        let ns = unsafe { DISCO_NS.as_ref() }.expect("[disco] test state missing");

        let manifest = [
            ManifestEntry {
                key: NODE_ID_KEY,
                flags: 0,
                value: ManifestValue::Bytes(b"disco-a"),
            },
            ManifestEntry {
                key: CLUSTER_KEY,
                flags: 0,
                value: ManifestValue::Bytes(b"test-cluster"),
            },
        ];

        let domain = spawn_disco(&manifest, ns);
        let status_page: *const u32 = {
            let base: *mut u8 = domain.status_frame.into();
            base as *const u32
        };
        logln!("[disco] service spawned (asid={})", domain.asid);

        // Wait for disco to reach the serving stage (>= 5).
        let mut spins: u64 = 0;
        let deadline = crate::self_test::results::Deadline::after_millis(30_000);
        while unsafe { core::ptr::read_volatile(status_page) } < 5 {
            spins += 1;
            if spins.is_multiple_of(2_000_000) {
                let stage = unsafe { core::ptr::read_volatile(status_page) };
                logln!("[disco] waiting: stage {}", stage);
            }
            deadline.assert_pending("EL0 disco service startup");
            yield_lp();
        }
        logln!("[disco] service reached serving stage.");

        // Optionally wait for cross-node peer discovery.
        #[cfg(feature = "disco_cross_node_test")]
        {
            let mut spins: u64 = 0;
            let deadline = crate::self_test::results::Deadline::after_millis(120_000);
            while unsafe { core::ptr::read_volatile(status_page.add(2)) } == 0 {
                spins += 1;
                if spins.is_multiple_of(2_000_000) {
                    let stage = unsafe { core::ptr::read_volatile(status_page) };
                    let peers = unsafe { core::ptr::read_volatile(status_page.add(2)) };
                    let rx_raw = unsafe { core::ptr::read_volatile(status_page.add(3)) };
                    let sent_ok = unsafe { core::ptr::read_volatile(status_page.add(4)) };
                    let sent_fail = unsafe { core::ptr::read_volatile(status_page.add(5)) };
                    let decoded = unsafe { core::ptr::read_volatile(status_page.add(6)) };
                    let called = unsafe { core::ptr::read_volatile(status_page.add(7)) };
                    let hb = unsafe { core::ptr::read_volatile(status_page.add(9)) };
                    let send_progress = unsafe { core::ptr::read_volatile(status_page.add(10)) };
                    let frouter_base =
                        unsafe { crate::self_test::FROUTER_STATUS_FRAME } as *const u32;
                    let frouter_rx = if frouter_base.is_null() {
                        0
                    } else {
                        unsafe { core::ptr::read_volatile(frouter_base.add(1)) }
                    };
                    let frouter_fwd = if frouter_base.is_null() {
                        0
                    } else {
                        unsafe { core::ptr::read_volatile(frouter_base.add(2)) }
                    };
                    let frouter_routes = if frouter_base.is_null() {
                        0
                    } else {
                        unsafe { core::ptr::read_volatile(frouter_base.add(5)) }
                    };
                    logln!(
                        "[disco] waiting: stage={} peers={} rx={} tx_ok={} tx_fail={} decoded={} \
                         call={} hb={} send={} frouter rx={} fwd={} routes={}",
                        stage,
                        peers,
                        rx_raw,
                        sent_ok,
                        sent_fail,
                        decoded,
                        called,
                        hb,
                        send_progress,
                        frouter_rx,
                        frouter_fwd,
                        frouter_routes
                    );
                }
                deadline.assert_pending("EL0 disco cross-node peer");
                yield_lp();
            }
            let peers = unsafe { core::ptr::read_volatile(status_page.add(2)) };
            logln!("[disco] discovered {} peer(s) on the network.", peers);
            assert!(peers > 0, "[disco] cross-node test requires at least one peer");
        }

        logln!("[disco] SUCCESS: cluster discovery service is running.");
        crate::self_test::results::pass(crate::self_test::results::TestId::Disco);
    }
}

pub use inner::test_el0_disco;
