//! Serialized ownership probes; cached acknowledgements avoid fixture pages.
use super::*;

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
