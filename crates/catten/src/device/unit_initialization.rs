//! One boot initialization claim in the existing typed backend slot.
//! No lazy initialization, shutdown, recovery registry or hardware retry.
mod tests;

use super::{
    detached_domain::DetachedDomain,
    dma::Error,
    dma_tables::Tables,
};
use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(super) enum UnitState<T> {
    Vacant,
    Claimed,
    Installed(T),
}

impl<T> UnitState<T> {
    pub(super) fn installed(&mut self) -> Result<&mut T, Error> {
        match self {
            Self::Vacant => Err(Error::Unsupported),
            Self::Claimed => Err(Error::OperationInFlight),
            Self::Installed(unit) => Ok(unit),
        }
    }
}

pub(super) trait UnitBacking {
    fn tables(&mut self) -> &mut Tables;
}

pub(super) struct Rejected<T> {
    error: Error,
    owner: Option<DetachedDomain<T>>,
    rollback_private: bool,
}

impl<T> Rejected<T> {
    pub(super) fn before_backing(error: Error) -> Self {
        Self {
            error,
            owner: None,
            rollback_private: true,
        }
    }

    pub(super) fn with_owner(error: Error, owner: DetachedDomain<T>) -> Self {
        Self {
            error,
            owner: Some(owner),
            rollback_private: true,
        }
    }

    /// A hardware control write has started. Even a still-private table owner
    /// is not proof that the command/control state may be initialized again.
    pub(super) fn retain_owner(error: Error, owner: DetachedDomain<T>) -> Self {
        Self {
            error,
            owner: Some(owner),
            rollback_private: false,
        }
    }
}

/// The claim stays installed on abandonment, including interruption before
/// allocation. Its payload's implicit field destruction is also inert.
#[must_use]
struct Claim<'a, T> {
    slot: &'a Mutex<UnitState<T>>,
    owner: Option<DetachedDomain<T>>,
}

impl<T: UnitBacking> Claim<'_, T> {
    fn cancel_private(mut self, cancel: impl FnOnce(&mut Tables) -> Result<(), Error>) {
        if let Some(owner) = self.owner.as_mut() {
            tests::probe();
            if cancel(owner.value_mut().tables()).is_err() {
                // Published, uncertain or physically partial: keep the claim
                // and complete payload. No destructor or subsequent retry.
                return;
            }
            tests::probe();
            drop(self.owner.take().unwrap().into_inner());
        }
        let mut slot = self.slot.lock();
        assert!(matches!(*slot, UnitState::Claimed));
        *slot = UnitState::Vacant;
    }

    fn install(mut self) {
        tests::probe();
        assert!(self.owner.as_mut().unwrap().value_mut().tables().is_published());
        let mut slot = self.slot.lock();
        assert!(matches!(*slot, UnitState::Claimed));
        *slot = UnitState::Installed(self.owner.take().unwrap().into_inner());
    }
}

pub(super) fn initialize<T: UnitBacking>(
    slot: &Mutex<UnitState<T>>,
    prepare: impl FnOnce() -> Result<DetachedDomain<T>, Rejected<T>>,
    ready: impl FnOnce(&T) -> bool,
) -> Result<(), Error> {
    {
        let mut state = slot.lock();
        match &*state {
            UnitState::Installed(unit) => {
                return if ready(unit) {
                    Ok(())
                } else {
                    Err(Error::OperationInFlight)
                };
            }
            UnitState::Claimed => return Err(Error::OperationInFlight),
            UnitState::Vacant => *state = UnitState::Claimed,
        }
    }
    let mut claim = Claim {
        slot,
        owner: None,
    };
    tests::probe();
    match prepare() {
        Ok(owner) => {
            claim.owner = Some(owner);
            claim.install();
            Ok(())
        }
        Err(Rejected {
            error,
            owner,
            rollback_private,
        }) => {
            claim.owner = owner;
            if rollback_private {
                claim.cancel_private(Tables::cancel_private);
            }
            Err(error)
        }
    }
}

// Serialized boot hooks. They never enable IRQs, fake hardware completion or
// permit published backing to use private cancellation.
pub(super) use tests::{
    publication_boundary,
    reject_allocation,
    reject_complete,
    test_real,
    wait_boundary,
};

pub(super) fn test_admission() {
    tests::run();
}
