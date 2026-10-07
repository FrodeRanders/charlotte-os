//! Serialized ownership probes; cached acknowledgements avoid fixture pages.
use super::*;

pub(crate) fn test_rejected_thread_abort(stale: ServiceDomain) {
    let mut phase = ShutdownPhaseSpec::one(ShutdownPhase::HttpIngress, stale);
    // Drain publication was already requested in this fixture state. Dummy
    // pages cannot authorize a new force publication after stale-root rejection.
    phase.domains[0].request_published = true;
    let mut node = NodeShutdownCoordinator::new(0, 0, alloc::vec![phase], alloc::vec![]);
    for _ in 0..2 {
        assert_eq!(
            node.poll(),
            NodeShutdownProgress::ReclamationFailed {
                phase: ShutdownPhase::HttpIngress,
                error: DomainTeardownError::ThreadAbortRejected,
                remaining_domains: 1,
            }
        );
        assert!(node.phases[0].domains[0].abort_failed);
        assert!(!node.phases[0].domains[0].force_requested);
        assert_eq!(node.phase_outcome(ShutdownPhase::HttpIngress), ShutdownPhaseOutcome::default());
        assert!(node.take_device_domains().is_none());
    }
    // Drop observes the cached failure and performs no retry/status write.
    drop(node);
    let counter = supervisor::DEPLOYMENT_FORCED_RETIREMENTS.load(Ordering::Relaxed);
    let registry = crate::cpu::multiprocessor::spin::mutex::Mutex::new(alloc::vec![
        supervisor::DeployedDomain {
            principal: 0x6162_6f72_745f_7465,
            domain: stale,
            shutdown_grace_ms: 0,
            retirement_deadline_ms: Some(0),
            retirement_reason: charlotte_launch::lifecycle::REASON_DEPLOYMENT_RETIRED,
            force_requested: false,
            retirement_acknowledged: false,
            teardown: supervisor::DeploymentTeardown::NotStarted,
        },
    ]);
    for _ in 0..2 {
        assert_eq!(
            crate::syscall::retire_deployed_artifact_with_registry(
                &registry,
                0x6162_6f72_745f_7465,
                true,
                charlotte_launch::lifecycle::REASON_DEPLOYMENT_RETIRED,
                0,
            ),
            u64::MAX
        );
        let entries = registry.lock();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].force_requested);
        assert!(matches!(
            entries[0].teardown,
            supervisor::DeploymentTeardown::Failed(DomainTeardownError::ThreadAbortRejected)
        ));
    }
    assert_eq!(supervisor::DEPLOYMENT_FORCED_RETIREMENTS.load(Ordering::Relaxed), counter);
}

pub(crate) fn test_failed_reclamation(domain: ServiceDomain) {
    let mut phase = ShutdownPhaseSpec::one(ShutdownPhase::HttpIngress, domain);
    phase.domains[0].teardown = Some(DomainTeardown::new(domain));
    phase.domains[0].lifecycle_status = Some(charlotte_launch::lifecycle::STATUS_READY);
    let mut node = NodeShutdownCoordinator::new(
        0,
        0,
        alloc::vec![phase],
        alloc::vec![DeviceShutdownDomain::new(DeviceShutdownKind::EntropyDriver, domain)],
    );
    let error =
        DomainTeardownError::AddressSpace(crate::memory::AddressSpaceCloseError::CloseInProgress);
    for _ in 0..2 {
        assert_eq!(
            node.poll(),
            NodeShutdownProgress::ReclamationFailed {
                phase: ShutdownPhase::HttpIngress,
                error,
                remaining_domains: 1,
            }
        );
        assert_eq!(node.phase_outcome(ShutdownPhase::HttpIngress), ShutdownPhaseOutcome::default());
        assert!(node.take_device_domains().is_none());
    }
    let mut device = DeviceShutdownDomain::new(DeviceShutdownKind::EntropyDriver, domain);
    // The acknowledgement was already checked in this fixture state. No dummy
    // status page is read and no hardware-quiescence claim is made by the test.
    device.teardown = Some(DomainTeardown::new(domain));
    let mut devices = DeviceShutdownCoordinator::new(0, alloc::vec![device]);
    for _ in 0..2 {
        assert_eq!(
            devices.poll(),
            DeviceShutdownProgress::ReclamationFailed {
                kind: DeviceShutdownKind::EntropyDriver,
                error,
            }
        );
    }
    test_poll_claim();
    crate::logln!(
        "[shutdown reclamation] terminal error retains phase/device gating and counters; \
         coordinator poll releases registry and rejects competing claim"
    );
}

fn test_poll_claim() {
    let slot = crate::cpu::multiprocessor::spin::mutex::Mutex::new(CoordinatorSlot {
        coordinator: Some(NodeShutdownCoordinator::new(0, 0, alloc::vec![], alloc::vec![])),
        polling: false,
    });
    assert_eq!(
        poll_shutdown_slot_with(&slot, |coordinator| {
            assert!(slot.try_lock().is_some(), "coordinator guard survived into poll");
            assert_eq!(
                poll_shutdown_slot_with(&slot, |_| panic!("competing caller polled owner")),
                Some(NodeShutdownProgress::Polling)
            );
            assert!(slot.lock().coordinator.is_none());
            coordinator.poll()
        }),
        Some(NodeShutdownProgress::AwaitingDeviceQuiescence {
            device_domains: 0
        })
    );
    assert!(!slot.lock().polling);
    assert!(slot.lock().coordinator.is_some());
}
