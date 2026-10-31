//! Serialized ownership probes; cached acknowledgements avoid fixture pages.
use core::sync::atomic::Ordering;

use super::*;

pub(crate) fn test_rejected_thread_abort(stale: ServiceDomain) {
    let mut phase = ShutdownPhaseSpec::one(ShutdownPhase::HttpIngress, stale);
    // Drain publication was already requested in this fixture state. Dummy
    // pages cannot authorize a new force publication after stale-root rejection.
    phase.domains[0].request_published = true;
    let mut node = NodeShutdownCoordinator::new(0, 0, [phase], []);
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
    // Abandonment retains the complete plan without retry/status writes.
    drop(node);
    let counter = supervisor::DEPLOYMENT_FORCED_RETIREMENTS.load(Ordering::Relaxed);
    let registry = crate::cpu::multiprocessor::spin::mutex::Mutex::new(
        supervisor::DeployedRegistry::<1>::new(),
    );
    registry.lock().insert(
        0x6162_6f72_745f_7465,
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
    );
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
    test_fixed_plan(domain);
    test_rejected_service_pages(domain);
    let mut phase = ShutdownPhaseSpec::one(ShutdownPhase::HttpIngress, domain);
    phase.domains[0].teardown = Some(DomainTeardown::new(domain));
    phase.domains[0].lifecycle_status = Some(charlotte_launch::lifecycle::STATUS_READY);
    let mut node = NodeShutdownCoordinator::new(
        0,
        0,
        [phase],
        [DeviceShutdownDomain::new(DeviceShutdownKind::EntropyDriver, domain)],
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
    let mut devices = DeviceShutdownCoordinator::new(0, [device]);
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
        coordinator: Some(NodeShutdownCoordinator::new(0, 0, [], [])),
        polling: false,
        deadline_ms: None,
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

/// Stale or closing roots reject status/drain access and cache that failure.
/// The caller checks that successor bytes and hardware authority are unchanged.
pub(crate) fn test_rejected_service_pages(domain: ServiceDomain) {
    let mut node = NodeShutdownCoordinator::new(
        u64::MAX,
        MAX_PHASE_GRACE_MS,
        [ShutdownPhaseSpec::one(ShutdownPhase::HttpIngress, domain)],
        [DeviceShutdownDomain::new(DeviceShutdownKind::EntropyDriver, domain)],
    );
    for _ in 0..2 {
        assert_eq!(
            node.poll(),
            NodeShutdownProgress::ReclamationFailed {
                phase: ShutdownPhase::HttpIngress,
                error: DomainTeardownError::ServicePagesUnavailable,
                remaining_domains: 1,
            }
        );
        assert!(node.phases[0].domains[0].page_failed);
        assert!(!node.phases[0].domains[0].request_published);
        assert!(!node.phases[0].domains[0].force_requested);
        assert!(node.take_device_domains().is_none());
        assert_eq!(node.phase_outcome(ShutdownPhase::HttpIngress), ShutdownPhaseOutcome::default());
    }
    drop(node);
    let mut devices = DeviceShutdownCoordinator::new(
        u64::MAX,
        [DeviceShutdownDomain::new(DeviceShutdownKind::EntropyDriver, domain)],
    );
    for _ in 0..2 {
        assert_eq!(
            devices.poll(),
            DeviceShutdownProgress::ReclamationFailed {
                kind: DeviceShutdownKind::EntropyDriver,
                error: DomainTeardownError::ServicePagesUnavailable,
            }
        );
        assert!(devices.domains[0].page_failed);
        assert!(!devices.domains[0].request_published);
        assert!(devices.domains[0].teardown.is_none());
    }
    let counter = supervisor::DEPLOYMENT_ACKNOWLEDGED_RETIREMENTS.load(Ordering::Relaxed);
    let registry = crate::cpu::multiprocessor::spin::mutex::Mutex::new(
        supervisor::DeployedRegistry::<1>::new(),
    );
    registry.lock().insert(
        0x7061_6765_7465_7374,
        supervisor::DeployedDomain {
            principal: 0x7061_6765_7465_7374,
            domain,
            shutdown_grace_ms: 60_000,
            retirement_deadline_ms: None,
            retirement_reason: 0,
            force_requested: false,
            retirement_acknowledged: false,
            teardown: supervisor::DeploymentTeardown::NotStarted,
        },
    );
    for _ in 0..2 {
        assert_eq!(
            crate::syscall::retire_deployed_artifact_with_registry(
                &registry,
                0x7061_6765_7465_7374,
                false,
                charlotte_launch::lifecycle::REASON_DEPLOYMENT_RETIRED,
                0,
            ),
            u64::MAX
        );
        let entries = registry.lock();
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].teardown,
            supervisor::DeploymentTeardown::Failed(DomainTeardownError::ServicePagesUnavailable)
        ));
        assert!(!entries[0].retirement_acknowledged);
        assert!(!entries[0].force_requested);
        assert!(entries[0].retirement_deadline_ms.is_none());
    }
    assert_eq!(supervisor::DEPLOYMENT_ACKNOWLEDGED_RETIREMENTS.load(Ordering::Relaxed), counter);
}

/// Exercise the actual maximum production plan, transfer and both parent
/// fallbacks beneath the locks that the former constructors/Drop reentered.
fn test_fixed_plan(domain: ServiceDomain) {
    use crate::{
        cpu::scheduler::threads::MASTER_THREAD_TABLE,
        memory::{
            ADDRESS_SPACE_LIFECYCLE,
            ADDRESS_SPACE_TABLE,
            allocators::global_allocator::PRIMARY_ALLOCATOR,
        },
        service::launch::{
            Cluster,
            DeploymentPlane,
            NetworkAppliance,
            NetworkStack,
            StorageStack,
        },
    };
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let state = SteadyState {
        telemetry: Some(domain),
        deployment: Some(DeploymentPlane {
            ingress: domain,
            clusterctl: domain,
            agent: domain,
        }),
        appliance: Some(NetworkAppliance {
            httpd: domain,
            time: domain,
            tcpip: domain,
        }),
        cluster: Some(Cluster {
            dns: domain,
            relmsg: domain,
            disco: domain,
        }),
        network: Some(NetworkStack {
            frouter: domain,
            driver: domain,
            driver_elf: b"net",
        }),
        storage: Some(StorageStack {
            objstore: domain,
            driver: domain,
            driver_elf: b"nvme",
        }),
        entropy: Some(domain),
    };
    // This is already a terminal fixture root. A prepared teardown exercises
    // the containing fallback too, without manufacturing a fresh close owner.
    let pending = DomainTeardown::new(domain);
    {
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let _roots = ADDRESS_SPACE_TABLE.lock();
        let _threads = MASTER_THREAD_TABLE.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        let mut plan = NodeShutdownCoordinator::from_steady_state(state, 100, 10);
        assert_eq!(plan.phases.len(), SHUTDOWN_PHASE_COUNT);
        assert_eq!(plan.device_domains.len(), MAX_DEVICE_DOMAINS);
        for index in 0..SHUTDOWN_PHASE_COUNT {
            assert_eq!(plan.phases[index].phase as usize, index);
            assert_eq!(plan.phases[index].domains.len(), 1);
        }
        assert!(plan.take_device_domains().is_none());
        plan.phases[0].domains[0].teardown = Some(pending);
        drop(plan);

        let mut plan = NodeShutdownCoordinator::new(
            100,
            10,
            [],
            [DeviceShutdownDomain::new(DeviceShutdownKind::EntropyDriver, domain)],
        );
        let devices = plan.take_device_domains().unwrap();
        assert_eq!(devices.len(), 1);
        assert!(plan.take_device_domains().is_none());
        drop(plan);
        drop(DeviceShutdownCoordinator::new(100, devices));
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[shutdown storage] full 12-phase/3-device plan, one-time transfer and inert parent \
         abandonment passed beneath lifecycle/root/thread/heap guards; node_bytes={} \
         device_bytes={}",
        core::mem::size_of::<NodeShutdownCoordinator>(),
        core::mem::size_of::<DeviceShutdownCoordinator>()
    );
}

#[cfg(feature = "shutdown_test")]
pub(crate) use worker::test_worker_admission;

#[cfg(feature = "shutdown_test")]
mod worker {
    use super::*;

    // Scheduled fixture: use the production transaction against private roster/
    // coordinator storage. The worker reads its actual published input on entry.
    static WORKER_SLOT: crate::cpu::multiprocessor::spin::mutex::Mutex<CoordinatorSlot> =
        crate::cpu::multiprocessor::spin::mutex::Mutex::new(CoordinatorSlot {
            coordinator: None,
            polling: false,
            deadline_ms: None,
        });
    static WORKER_OBSERVATION: core::sync::atomic::AtomicUsize =
        core::sync::atomic::AtomicUsize::new(0);

    extern "C" fn admission_worker() {
        let slot = WORKER_SLOT.lock();
        let valid = slot.deadline_ms == Some(123_456)
            && slot.coordinator.as_ref().is_some_and(|owner| owner.node_deadline_ms == 123_456);
        WORKER_OBSERVATION.store(
            if valid {
                1
            } else {
                2
            },
            Ordering::Release,
        );
    }

    pub(crate) fn test_worker_admission() {
        use crate::{
            cpu::{
                multiprocessor::spin::mutex::Mutex,
                scheduler::{
                    prepared::PreparedKernelThread,
                    system_scheduler::Error,
                    threads::{
                        MASTER_THREAD_TABLE,
                        ThreadState,
                    },
                },
            },
            memory::{
                ADDRESS_SPACE_TABLE,
                allocators::global_allocator::PRIMARY_ALLOCATOR,
            },
        };
        let roster = Mutex::new(Some(SteadyState {
            telemetry: None,
            storage: None,
            entropy: None,
            network: None,
            cluster: None,
            appliance: None,
            deployment: None,
        }));
        let irq = crate::cpu::isa::lp::ops::get_int_state();
        assert_eq!(
            start_worker_with(&WORKER_SLOT, &roster, 123_456, MAX_PHASE_GRACE_MS + 1, || panic!(
                "invalid grace allocated a worker"
            )),
            Err(BeginNodeShutdownError::InvalidPhaseGrace)
        );
        assert_eq!(
            start_worker_with(&WORKER_SLOT, &roster, 123_456, 10, || {
                assert!(WORKER_SLOT.try_lock().is_some());
                assert!(roster.try_lock().is_some());
                Err(Error::ThreadPreparationFailed)
            }),
            Err(BeginNodeShutdownError::WorkerAdmissionFailed)
        );
        assert!(roster.lock().is_some());
        assert!(WORKER_SLOT.lock().coordinator.is_none());
        assert_eq!(WORKER_SLOT.lock().deadline_ms, None);

        let mut rejected = None;
        assert_eq!(
            start_worker_with(&WORKER_SLOT, &roster, 123_456, 10, || {
                let worker = PreparedKernelThread::try_new(admission_worker)?;
                let identity = worker.identity();
                rejected = Some(identity);
                MASTER_THREAD_TABLE
                    .write()
                    .get_mut(identity.0)
                    .unwrap()
                    .abort_requested
                    .store(true, Ordering::Release);
                Ok(worker)
            }),
            Err(BeginNodeShutdownError::WorkerAdmissionFailed)
        );
        let (tid, generation) = rejected.unwrap();
        assert!(
            !MASTER_THREAD_TABLE
                .read()
                .get(tid)
                .is_ok_and(|thread| thread.generation == generation)
        );
        assert!(roster.lock().is_some());
        assert!(WORKER_SLOT.lock().coordinator.is_none());
        assert_eq!(WORKER_OBSERVATION.load(Ordering::Acquire), 0);

        let mut absent = None;
        assert_eq!(
            start_worker_with(&WORKER_SLOT, &Mutex::new(None), 123_456, 10, || {
                let worker = PreparedKernelThread::try_new(admission_worker)?;
                absent = Some(worker.identity());
                Ok(worker)
            }),
            Err(BeginNodeShutdownError::SteadyStateUnavailable)
        );
        let (tid, generation) = absent.unwrap();
        assert!(
            !MASTER_THREAD_TABLE
                .read()
                .get(tid)
                .is_ok_and(|thread| thread.generation == generation)
        );

        // A contender wins after the early precheck but before final admission.
        let mut loser = None;
        assert_eq!(
            start_worker_with(&WORKER_SLOT, &roster, 123_456, 10, || {
                let worker = PreparedKernelThread::try_new(admission_worker)?;
                loser = Some(worker.identity());
                WORKER_SLOT.lock().coordinator = Some(NodeShutdownCoordinator::new(0, 0, [], []));
                Ok(worker)
            }),
            Err(BeginNodeShutdownError::AlreadyInProgress)
        );
        let (tid, generation) = loser.unwrap();
        assert!(
            !MASTER_THREAD_TABLE
                .read()
                .get(tid)
                .is_ok_and(|thread| thread.generation == generation)
        );
        assert!(roster.lock().is_some());
        assert_eq!(WORKER_OBSERVATION.load(Ordering::Acquire), 0);
        drop(WORKER_SLOT.lock().coordinator.take());

        // Abandonment keeps the existing exact table payload, even beneath roots
        // and allocator serialization. It is deliberately not reconstructed later.
        let abandoned = PreparedKernelThread::try_new(admission_worker).unwrap();
        let (tid, generation) = abandoned.identity();
        {
            let _roots = ADDRESS_SPACE_TABLE.lock();
            let table = MASTER_THREAD_TABLE.read();
            let _heap = PRIMARY_ALLOCATOR.lock();
            drop(abandoned);
            assert_eq!(table.get(tid).unwrap().generation, generation);
            assert!(matches!(table.get(tid).unwrap().state, ThreadState::NeedsLpAssignment));
        }

        assert_eq!(
            start_worker_with(
                &WORKER_SLOT,
                &roster,
                123_456,
                10,
                || PreparedKernelThread::try_new(admission_worker)
            ),
            Ok(())
        );
        assert!(roster.lock().is_none());
        assert_eq!(WORKER_SLOT.lock().deadline_ms, Some(123_456));
        assert_eq!(
            start_worker_with(&WORKER_SLOT, &roster, 123_456, 10, || panic!(
                "duplicate shutdown allocated a worker"
            )),
            Err(BeginNodeShutdownError::AlreadyInProgress)
        );
        let deadline = crate::self_test::results::Deadline::after_millis(5_000);
        while WORKER_OBSERVATION.load(Ordering::Acquire) == 0 {
            deadline.assert_pending("shutdown admission worker observation");
            crate::cpu::scheduler::yield_lp();
        }
        assert_eq!(WORKER_OBSERVATION.load(Ordering::Acquire), 1);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
        crate::logln!(
            "[shutdown worker] preparation/scheduler rejection, absent roster, competing winner, \
             inert abandonment and input-before-dispatch passed"
        );
    }
}
