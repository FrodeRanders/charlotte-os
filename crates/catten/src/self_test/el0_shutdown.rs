//! Isolated end-to-end verification of the protected-domain lifecycle ABI.

use crate::{
    cpu::scheduler::{
        monotonic_millis,
        sleep_millis,
        system_scheduler::SYSTEM_SCHEDULER,
    },
    ipc::ConnectionRights,
    logln,
    service::{
        bootstrap::{
            self,
            ManifestEntry,
            ManifestValue,
        },
        supervisor::{
            self,
            ServiceDomain,
        },
    },
};

const STATUS_STARTED: usize = 0;
const MODE_KEY: u64 = 0x7368_7574_6d6f_6465; // "shutmode"

struct TestState {
    cooperative: ServiceDomain,
    stubborn: ServiceDomain,
}

static TEST_STATE: spin::Mutex<Option<TestState>> = spin::Mutex::new(None);

pub fn test_el0_shutdown() {
    logln!("Testing cooperative and forced protected-domain shutdown...");
    let image = crate::service::store::service_elf(b"shutdown_probe")
        .expect("[shutdown] shutdown_probe.elf");
    let name_service = supervisor::node_name_service();
    let cooperative =
        supervisor::spawn_with_name_service(image, &name_service, ConnectionRights::CALL);
    let stubborn = supervisor::spawn_with_manifest(
        image,
        &name_service,
        ConnectionRights::CALL,
        &[ManifestEntry {
            key: MODE_KEY,
            flags: 0,
            value: ManifestValue::Unsigned(1),
        }],
    );
    *TEST_STATE.lock() = Some(TestState {
        cooperative,
        stubborn,
    });
    crate::self_test::results::spawn_verifier(
        crate::self_test::results::TestId::Shutdown,
        verify_el0_shutdown,
    );
}

fn wait_started(domain: &ServiceDomain, what: &str) {
    let deadline = crate::self_test::results::Deadline::after_millis(10_000);
    loop {
        let base: *const u8 = domain.status_frame.into();
        if unsafe { crate::self_test::status_u32(base, STATUS_STARTED) } == 1 {
            return;
        }
        deadline.assert_pending(what);
        sleep_millis(10);
    }
}

extern "C" fn verify_el0_shutdown() {
    let TestState {
        cooperative,
        stubborn,
    } = TEST_STATE.lock().take().expect("[shutdown] verifier state");

    wait_started(&cooperative, "cooperative probe startup");
    let deadline = monotonic_millis().saturating_add(2_000);
    bootstrap::write_lifecycle_request(
        cooperative.config_frame,
        charlotte_launch::lifecycle::STATE_DRAIN_REQUESTED,
        charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
        deadline,
    );
    supervisor::wait_domain_exit(&cooperative, 10_000);
    assert_eq!(
        bootstrap::lifecycle_status(cooperative.status_frame),
        charlotte_launch::lifecycle::STATUS_READY,
        "cooperative probe exited without acknowledging cleanup"
    );
    supervisor::teardown_domain(cooperative);
    logln!("[shutdown] cooperative cleanup acknowledged and domain reclaimed");

    wait_started(&stubborn, "stubborn probe startup");
    let deadline = monotonic_millis().saturating_add(100);
    bootstrap::write_lifecycle_request(
        stubborn.config_frame,
        charlotte_launch::lifecycle::STATE_DRAIN_REQUESTED,
        charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
        deadline,
    );
    sleep_millis(150);
    assert!(
        !supervisor::domain_exited(&stubborn),
        "stubborn probe unexpectedly honored the drain request"
    );
    bootstrap::write_lifecycle_request(
        stubborn.config_frame,
        charlotte_launch::lifecycle::STATE_FORCE_TERMINATING,
        charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
        deadline,
    );
    SYSTEM_SCHEDULER.read().abort_as_threads(stubborn.asid);
    supervisor::wait_domain_exit(&stubborn, 10_000);
    supervisor::teardown_domain(stubborn);
    logln!("[shutdown] unresponsive domain forcibly terminated and reclaimed");

    logln!("[shutdown] SUCCESS: bounded cooperative and forced shutdown verified");
    crate::self_test::results::pass(crate::self_test::results::TestId::Shutdown);
}
