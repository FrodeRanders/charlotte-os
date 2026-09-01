//! Isolated end-to-end verification of the protected-domain lifecycle ABI.

use crate::{
    cpu::scheduler::{
        monotonic_millis,
        sleep_millis,
    },
    ipc::ConnectionRights,
    logln,
    service::{
        bootstrap::{
            self,
            ManifestEntry,
            ManifestValue,
        },
        shutdown::{
            DeviceShutdownCoordinator,
            DeviceShutdownDomain,
            DeviceShutdownKind,
            DeviceShutdownProgress,
            NodeShutdownCoordinator,
            NodeShutdownProgress,
            ShutdownPhase,
            ShutdownPhaseSpec,
            begin_device_shutdown,
            begin_node_shutdown,
            poll_node_shutdown,
        },
        supervisor::{
            self,
            ServiceDomain,
        },
    },
};

const STATUS_STARTED: usize = 0;
const MODE_KEY: u64 = 0x7368_7574_6d6f_6465; // "shutmode"
const COOPERATIVE_PRINCIPAL: u64 = 0x7368_7574_0000_0001;
const STUBBORN_PRINCIPAL: u64 = 0x7368_7574_0000_0002;

struct TestState {
    cooperative: ServiceDomain,
    stubborn: ServiceDomain,
    acknowledged_before: u64,
    forced_before: u64,
    phased: [ServiceDomain; 3],
    device: ServiceDomain,
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
    let phased = [
        supervisor::spawn_with_name_service(image, &name_service, ConnectionRights::CALL),
        supervisor::spawn_with_name_service(image, &name_service, ConnectionRights::CALL),
        supervisor::spawn_with_name_service(image, &name_service, ConnectionRights::CALL),
    ];
    let device = supervisor::spawn_with_manifest(
        image,
        &name_service,
        ConnectionRights::CALL,
        &[ManifestEntry {
            key: MODE_KEY,
            flags: 0,
            value: ManifestValue::Unsigned(2),
        }],
    );
    let acknowledged_before = supervisor::NODE_SHUTDOWN_ACKNOWLEDGED_RETIREMENTS
        .load(core::sync::atomic::Ordering::Relaxed);
    let forced_before =
        supervisor::NODE_SHUTDOWN_FORCED_RETIREMENTS.load(core::sync::atomic::Ordering::Relaxed);
    supervisor::DEPLOYED_DOMAINS.lock().extend([
        supervisor::DeployedDomain {
            principal: COOPERATIVE_PRINCIPAL,
            domain: cooperative,
            shutdown_grace_ms: 2_000,
            retirement_deadline_ms: None,
            retirement_reason: charlotte_launch::lifecycle::REASON_DEPLOYMENT_RETIRED,
            force_requested: false,
        },
        supervisor::DeployedDomain {
            principal: STUBBORN_PRINCIPAL,
            domain: stubborn,
            shutdown_grace_ms: 100,
            retirement_deadline_ms: None,
            retirement_reason: charlotte_launch::lifecycle::REASON_DEPLOYMENT_RETIRED,
            force_requested: false,
        },
    ]);
    *TEST_STATE.lock() = Some(TestState {
        cooperative,
        stubborn,
        acknowledged_before,
        forced_before,
        phased,
        device,
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

fn lifecycle_request(domain: &ServiceDomain) -> (u32, u32, u64) {
    let base: *const u8 = domain.config_frame.into();
    unsafe {
        let state = core::ptr::read_volatile(
            base.add(charlotte_launch::lifecycle::CONTROL_STATE_OFFSET) as *const u32,
        );
        let reason = core::ptr::read_volatile(
            base.add(charlotte_launch::lifecycle::CONTROL_REASON_OFFSET) as *const u32,
        );
        let deadline = core::ptr::read_volatile(
            base.add(charlotte_launch::lifecycle::CONTROL_DEADLINE_MS_OFFSET) as *const u64,
        );
        (state, reason, deadline)
    }
}

extern "C" fn verify_el0_shutdown() {
    let TestState {
        cooperative,
        stubborn,
        acknowledged_before,
        forced_before,
        phased,
        device,
    } = TEST_STATE.lock().take().expect("[shutdown] verifier state");

    wait_started(&cooperative, "cooperative probe startup");
    let node_deadline = monotonic_millis().saturating_add(1_000);
    assert_eq!(
        crate::syscall::retire_deployed_artifact(
            COOPERATIVE_PRINCIPAL,
            false,
            charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
            node_deadline,
        ),
        1,
        "cooperative node retirement did not enter the draining state"
    );
    assert_eq!(
        lifecycle_request(&cooperative),
        (
            charlotte_launch::lifecycle::STATE_DRAIN_REQUESTED,
            charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
            node_deadline,
        ),
        "enclosing node deadline did not cap the child's signed grace"
    );
    supervisor::wait_domain_exit(&cooperative, 10_000);
    assert_eq!(
        bootstrap::lifecycle_status(cooperative.status_frame),
        charlotte_launch::lifecycle::STATUS_READY,
        "cooperative probe exited without acknowledging cleanup"
    );
    assert_eq!(
        crate::syscall::retire_deployed_artifact(
            COOPERATIVE_PRINCIPAL,
            false,
            charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
            node_deadline,
        ),
        0,
        "cooperative node retirement did not reclaim the domain"
    );
    logln!("[shutdown] cooperative cleanup acknowledged and domain reclaimed");

    wait_started(&stubborn, "stubborn probe startup");
    let child_grace_started = monotonic_millis();
    let node_deadline = monotonic_millis().saturating_add(10_000);
    assert_eq!(
        crate::syscall::retire_deployed_artifact(
            STUBBORN_PRINCIPAL,
            false,
            charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
            node_deadline,
        ),
        1,
        "stubborn node retirement did not enter the draining state"
    );
    let (state, reason, child_deadline) = lifecycle_request(&stubborn);
    let child_grace_observed = monotonic_millis();
    assert_eq!(state, charlotte_launch::lifecycle::STATE_DRAIN_REQUESTED);
    assert_eq!(reason, charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN);
    assert!(
        child_deadline >= child_grace_started
            && child_deadline <= child_grace_observed.saturating_add(100)
            && child_deadline < node_deadline,
        "signed child grace did not cap the enclosing node deadline"
    );
    sleep_millis(150);
    assert!(
        !supervisor::domain_exited(&stubborn),
        "stubborn probe unexpectedly honored the drain request"
    );
    assert_eq!(
        crate::syscall::retire_deployed_artifact(
            STUBBORN_PRINCIPAL,
            false,
            charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
            node_deadline,
        ),
        1,
        "expired child grace did not request forced termination"
    );
    supervisor::wait_domain_exit(&stubborn, 10_000);
    assert_eq!(
        crate::syscall::retire_deployed_artifact(
            STUBBORN_PRINCIPAL,
            false,
            charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
            node_deadline,
        ),
        0,
        "forced node retirement did not reclaim the domain"
    );
    assert_eq!(
        supervisor::NODE_SHUTDOWN_ACKNOWLEDGED_RETIREMENTS
            .load(core::sync::atomic::Ordering::Relaxed),
        acknowledged_before + 1,
        "cooperative node-shutdown outcome was not recorded"
    );
    assert_eq!(
        supervisor::NODE_SHUTDOWN_FORCED_RETIREMENTS.load(core::sync::atomic::Ordering::Relaxed),
        forced_before + 1,
        "forced node-shutdown outcome was not recorded"
    );
    logln!("[shutdown] unresponsive domain forcibly terminated and reclaimed");

    for domain in &phased {
        wait_started(domain, "phased node-service probe startup");
    }
    wait_started(&device, "device-quiescence probe startup");
    let node_deadline = monotonic_millis().saturating_add(10_000);
    let mut coordinator = NodeShutdownCoordinator::new(
        node_deadline,
        1_000,
        alloc::vec![
            ShutdownPhaseSpec::one(ShutdownPhase::HttpIngress, phased[0]),
            ShutdownPhaseSpec::one(ShutdownPhase::Time, phased[1]),
            ShutdownPhaseSpec::one(ShutdownPhase::ObjectStore, phased[2]),
        ],
        alloc::vec![DeviceShutdownDomain::new(DeviceShutdownKind::EntropyDriver, device)],
    );
    assert_eq!(
        coordinator.poll(),
        NodeShutdownProgress::Draining {
            phase: ShutdownPhase::HttpIngress,
            remaining_domains: 1,
        }
    );
    assert_eq!(lifecycle_request(&phased[0]).0, charlotte_launch::lifecycle::STATE_DRAIN_REQUESTED);
    assert_eq!(
        lifecycle_request(&phased[1]).0,
        charlotte_launch::lifecycle::STATE_RUNNING,
        "a dependent shutdown phase started before ingress drained"
    );

    supervisor::wait_domain_exit(&phased[0], 10_000);
    assert_eq!(
        coordinator.poll(),
        NodeShutdownProgress::Draining {
            phase: ShutdownPhase::Time,
            remaining_domains: 1,
        }
    );
    assert_eq!(
        lifecycle_request(&phased[2]).0,
        charlotte_launch::lifecycle::STATE_RUNNING,
        "storage shutdown started before its consumer drained"
    );

    supervisor::wait_domain_exit(&phased[1], 10_000);
    assert_eq!(
        coordinator.poll(),
        NodeShutdownProgress::Draining {
            phase: ShutdownPhase::ObjectStore,
            remaining_domains: 1,
        }
    );
    supervisor::wait_domain_exit(&phased[2], 10_000);
    assert_eq!(
        coordinator.poll(),
        NodeShutdownProgress::AwaitingDeviceQuiescence {
            device_domains: 1,
        }
    );
    let domains = coordinator
        .take_device_domains()
        .expect("device domains were not exposed after service drain completed");
    assert_eq!(coordinator.poll(), NodeShutdownProgress::DeviceDomainsTransferred);
    assert!(
        coordinator.take_device_domains().is_none(),
        "hardware-root domain ownership transferred more than once"
    );
    logln!("[shutdown] reverse dependency phases gated and reclaimed in order");

    let mut devices = DeviceShutdownCoordinator::new(node_deadline, domains);
    assert_eq!(
        devices.poll(),
        DeviceShutdownProgress::Quiescing {
            remaining_domains: 1,
        }
    );
    assert_eq!(
        lifecycle_request(&device).0,
        charlotte_launch::lifecycle::STATE_DRAIN_REQUESTED,
        "device quiescence request was not published"
    );
    supervisor::wait_domain_exit(&device, 10_000);
    assert_eq!(
        bootstrap::lifecycle_status(device.status_frame),
        charlotte_launch::lifecycle::STATUS_DEVICE_QUIESCED,
        "device probe used the ordinary service acknowledgement"
    );
    assert_eq!(devices.poll(), DeviceShutdownProgress::Complete);
    logln!("[shutdown] device domain required quiescence acknowledgement before reclamation");

    let production_deadline = monotonic_millis().saturating_add(10_000);
    // Most high-level platform services do not yet have cooperative cleanup.
    // Keep their isolated-test grace short enough to reach real device
    // quiescence before unrelated background verifiers complete.
    begin_node_shutdown(production_deadline, 100)
        .expect("production node shutdown did not acquire the steady-state service set");
    let production_wait = crate::self_test::results::Deadline::after_millis(10_000);
    let expected_devices = loop {
        match poll_node_shutdown().expect("production node shutdown coordinator disappeared") {
            NodeShutdownProgress::Draining {
                ..
            } => {
                production_wait.assert_pending("production node-service shutdown");
                sleep_millis(10);
            }
            NodeShutdownProgress::AwaitingDeviceQuiescence {
                device_domains,
            } => break device_domains,
            NodeShutdownProgress::DeviceDomainsTransferred => {
                panic!("device ownership transferred before the test acquired it")
            }
        }
    };
    assert!(expected_devices >= 2, "storage and entropy drivers were not retained");
    let mut production_devices = begin_device_shutdown(production_deadline)
        .expect("production device shutdown did not acquire hardware-root domains");
    loop {
        match production_devices.poll() {
            DeviceShutdownProgress::Quiescing {
                ..
            } => {
                production_wait.assert_pending("production device quiescence");
                sleep_millis(10);
            }
            DeviceShutdownProgress::Complete => break,
            other => panic!("production device quiescence failed: {:?}", other),
        }
    }
    logln!(
        "[shutdown] production object store and {} device adapter(s) flushed/reset and quiesced",
        expected_devices
    );

    logln!(
        "[shutdown] SUCCESS: bounded domain retirement and reverse-order service drain verified"
    );
    crate::self_test::results::pass(crate::self_test::results::TestId::Shutdown);
}
