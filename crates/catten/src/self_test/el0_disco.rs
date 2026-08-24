//! Self-test: cluster discovery service.
//!
//! Observes the node's operational discovery service and verifies it starts
//! successfully. When `disco_cross_node_test` is active, the verifier also
//! waits for cross-node peer discovery to confirm that the Ethernet broadcast
//! bootstrap protocol works end-to-end.

mod inner {
    use crate::logln;

    pub fn test_el0_disco() {
        logln!("Testing EL0 cluster discovery service...");

        let _vtid = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Disco,
            verify_el0_disco,
        );
        logln!("[disco] verifier deferred (waits for disco service + optional cross-node peer)");
    }

    extern "C" fn verify_el0_disco() {
        use crate::cpu::scheduler::yield_lp;

        let domain = crate::service::launch::steady_state()
            .cluster
            .expect("[disco] operational cluster services missing")
            .disco;
        let status_page: *const u8 = {
            let base: *mut u8 = domain.status_frame.into();
            base
        };
        let status_word = |offset| unsafe { crate::self_test::status_u32(status_page, offset) };
        logln!("[disco] observing operational service (asid={})", domain.asid);

        // Wait for disco to reach the serving stage (>= 5).
        let mut spins: u64 = 0;
        let deadline = crate::self_test::results::Deadline::after_millis(30_000);
        while status_word(charlotte_launch::disco_status::STAGE) < 5 {
            spins += 1;
            if spins.is_multiple_of(2_000_000) {
                let stage = status_word(charlotte_launch::disco_status::STAGE);
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
            while status_word(charlotte_launch::disco_status::PEER_COUNT) == 0 {
                spins += 1;
                if spins.is_multiple_of(2_000_000) {
                    let stage = status_word(charlotte_launch::disco_status::STAGE);
                    let peers = status_word(charlotte_launch::disco_status::PEER_COUNT);
                    let rx_raw = status_word(charlotte_launch::disco_status::RX_RAW);
                    let sent_ok = status_word(charlotte_launch::disco_status::SENT_OK);
                    let sent_fail = status_word(charlotte_launch::disco_status::SENT_FAIL);
                    let decoded = status_word(charlotte_launch::disco_status::DECODED);
                    let called = status_word(charlotte_launch::disco_status::CALLED);
                    let hb = status_word(charlotte_launch::disco_status::HEARTBEAT);
                    let send_progress = status_word(charlotte_launch::disco_status::SEND_PROGRESS);
                    let frouter_base = crate::self_test::FROUTER_STATUS_FRAME
                        .load(core::sync::atomic::Ordering::Acquire)
                        as *const u8;
                    let frouter_rx = if frouter_base.is_null() {
                        0
                    } else {
                        unsafe {
                            crate::self_test::status_u32(
                                frouter_base,
                                charlotte_launch::frouter_status::RX_TOTAL,
                            )
                        }
                    };
                    let frouter_fwd = if frouter_base.is_null() {
                        0
                    } else {
                        unsafe {
                            crate::self_test::status_u32(
                                frouter_base,
                                charlotte_launch::frouter_status::FORWARDED,
                            )
                        }
                    };
                    let frouter_routes = if frouter_base.is_null() {
                        0
                    } else {
                        unsafe {
                            crate::self_test::status_u32(
                                frouter_base,
                                charlotte_launch::frouter_status::ROUTES,
                            )
                        }
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
            let peers = status_word(charlotte_launch::disco_status::PEER_COUNT);
            logln!("[disco] discovered {} peer(s) on the network.", peers);
            assert!(peers > 0, "[disco] cross-node test requires at least one peer");
        }

        logln!("[disco] SUCCESS: cluster discovery service is running.");
        crate::self_test::results::pass(crate::self_test::results::TestId::Disco);
    }
}

pub use inner::test_el0_disco;
