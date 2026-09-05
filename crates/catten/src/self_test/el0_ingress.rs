//! Passive in-kernel observer for the external distributed-ingress fixture.
//!
//! A fourth stream-hub participant generates genuine off-node ARP and TCP
//! traffic. This observer keeps each survivor's self-test pending until it has
//! seen a stable three-backend snapshot, ingress traffic, and a replacement
//! VIP advertiser after the original owner disappears.

use crate::logln;

pub fn test_el0_ingress() {
    let _ = crate::self_test::results::spawn_verifier(
        crate::self_test::results::TestId::Ingress,
        verify_el0_ingress,
    );
    logln!("[ingress] passive verifier deferred; waiting for external L2 traffic and failover");
}

extern "C" fn verify_el0_ingress() {
    use charlotte_launch::frouter_status as status;

    use crate::cpu::scheduler::yield_lp;

    let deadline = crate::self_test::results::Deadline::after_millis(150_000);
    let frouter = loop {
        let frame = crate::self_test::FROUTER_STATUS_FRAME
            .load(core::sync::atomic::Ordering::Acquire) as *const u8;
        if !frame.is_null() {
            break frame;
        }
        deadline.assert_pending("frouter status publication");
        yield_lp();
    };

    let initial_advertiser = loop {
        let backends = unsafe { crate::self_test::status_u32(frouter, status::BACKENDS) };
        let advertiser = unsafe { crate::self_test::status_u32(frouter, status::VIP_ADVERTISER) };
        if backends >= 3 && advertiser != 0 {
            break advertiser;
        }
        deadline.assert_pending("stable three-member ingress snapshot");
        yield_lp();
    };

    loop {
        let local = unsafe { crate::self_test::status_u32(frouter, status::INGRESS_LOCAL) };
        let remote = unsafe { crate::self_test::status_u32(frouter, status::INGRESS_FORWARDED) };
        let advertiser = unsafe { crate::self_test::status_u32(frouter, status::VIP_ADVERTISER) };
        if local.saturating_add(remote) != 0 && advertiser != 0 && advertiser != initial_advertiser
        {
            logln!(
                "[ingress] external VIP failover observed: advertiser={:#x} local={} remote={}",
                advertiser,
                local,
                remote
            );
            break;
        }
        deadline.assert_pending("replacement VIP advertiser and external traffic");
        yield_lp();
    }

    crate::self_test::results::pass(crate::self_test::results::TestId::Ingress);
}
