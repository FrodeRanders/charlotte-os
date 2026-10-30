//! Reserve exact deployment identity and fixed storage before running a launch.

mod storage;
pub(crate) use storage::Registry;

use super::{
    DeployedDomain,
    DeploymentTeardown,
    ServiceDomain,
};
use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const MAX_DEPLOYED_DOMAINS: usize = 64;
pub(crate) type DeployedRegistry<const N: usize = MAX_DEPLOYED_DOMAINS> =
    Registry<DeployedDomain, N>;

pub(crate) static DEPLOYED_DOMAINS: Mutex<DeployedRegistry> = Mutex::new(Registry::new());

/// Its only fallback is retaining the exact reserved slot. No lock, callback,
/// allocation or physical cleanup runs on abandonment.
#[must_use]
pub(crate) struct PreparingDeployment<'a, const N: usize = MAX_DEPLOYED_DOMAINS> {
    registry: &'a Mutex<DeployedRegistry<N>>,
    reservation: storage::Reservation,
}

impl PreparingDeployment<'static> {
    pub(crate) fn reserve(principal: u64) -> Option<Self> {
        Self::reserve_in(&DEPLOYED_DOMAINS, principal)
    }
}

impl<'a, const N: usize> PreparingDeployment<'a, N> {
    fn reserve_in(registry: &'a Mutex<DeployedRegistry<N>>, principal: u64) -> Option<Self> {
        let reservation = registry.lock().reserve(principal).ok()?;
        Some(Self {
            registry,
            reservation,
        })
    }

    /// The callback runs once, with no registry guard. An ordinary rejected
    /// launch returns no running domain; cancellation releases bookkeeping
    /// only, never root backing, physical charges or an uncertain operation.
    pub(crate) fn launch(
        self,
        launch: impl FnOnce() -> Option<(ServiceDomain, u32)>,
    ) -> Option<ServiceDomain> {
        let Some((domain, shutdown_grace_ms)) = launch() else {
            self.registry.lock().cancel(self.reservation);
            return None;
        };
        let principal = self.reservation.principal();
        self.registry.lock().publish(
            self.reservation,
            DeployedDomain {
                principal,
                domain,
                shutdown_grace_ms,
                retirement_deadline_ms: None,
                retirement_reason: charlotte_launch::lifecycle::REASON_DEPLOYMENT_RETIRED,
                force_requested: false,
                teardown: DeploymentTeardown::NotStarted,
                retirement_acknowledged: false,
            },
        );
        Some(domain)
    }
}

/// Serialized kernel fixture: the root has no running thread or service pages.
/// Pending preparation must reject retirement before attempting page access.
pub(super) fn test_preparation(domain: ServiceDomain) {
    const PRINCIPAL: u64 = 0x6465_706c_7072_6570;
    let registry = Mutex::new(DeployedRegistry::<1>::new());
    let preparation = PreparingDeployment::reserve_in(&registry, PRINCIPAL).unwrap();
    assert!(PreparingDeployment::reserve_in(&registry, PRINCIPAL).is_none());
    assert!(PreparingDeployment::reserve_in(&registry, PRINCIPAL + 1).is_none());
    assert_eq!(
        crate::syscall::retire_deployed_artifact_with_registry(
            &registry,
            PRINCIPAL,
            true,
            charlotte_launch::lifecycle::REASON_DEPLOYMENT_RETIRED,
            0,
        ),
        1
    );
    let mut calls = 0;
    assert!(
        preparation
            .launch(|| {
                assert!(registry.try_lock().is_some());
                calls += 1;
                None
            })
            .is_none()
    );
    assert_eq!(calls, 1);
    assert!(!registry.lock().preparing(PRINCIPAL));
    let preparation = PreparingDeployment::reserve_in(&registry, PRINCIPAL).unwrap();
    let launched = preparation
        .launch(|| {
            assert!(registry.try_lock().is_some());
            calls += 1;
            Some((domain, 123))
        })
        .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(launched.address_space, domain.address_space);
    let retired = {
        let mut registry = registry.lock();
        let position = registry.position(|entry| entry.principal == PRINCIPAL).unwrap();
        assert_eq!(registry[position].shutdown_grace_ms, 123);
        assert_eq!(registry[position].domain.address_space, domain.address_space);
        registry.remove(position)
    };
    drop(retired);
    crate::memory::close_user_address_space_handle(domain.address_space).unwrap();
    crate::logln!(
        "[deployment registry] reserved capacity/identity, Pending retirement, unlocked one-shot \
         launch, rejection and publication passed"
    );
}
