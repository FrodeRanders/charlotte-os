//! Exclusive creation owns the complete installed unit in its existing slot.
//! This composes a borrow of the containing grant; neither fallback can release
//! roots, authority, endpoint claims, tables, queues, mappings or pins.
use super::{
    DmaCreation,
    detached_domain::DetachedDomain,
    dma::Error,
    unit_initialization::UnitState,
};
use crate::cpu::multiprocessor::spin::mutex::Mutex;

#[must_use]
struct Preparing<'slot, 'grant, T> {
    slot: &'slot Mutex<UnitState<T>>,
    unit: DetachedDomain<T>,
    grant: &'grant mut DmaCreation,
}

impl<'slot, 'grant, T> Preparing<'slot, 'grant, T> {
    fn begin(
        slot: &'slot Mutex<UnitState<T>>,
        grant: &'grant mut DmaCreation,
        ready: impl FnOnce(&T) -> bool,
    ) -> Result<Self, Error> {
        if grant.is_armed() {
            return Err(Error::OperationInFlight);
        }
        let mut state = slot.lock();
        if !ready(state.installed()?) {
            return Err(Error::OperationInFlight);
        }
        // No allocation, callback or fallible work between extraction and the
        // complete retaining owner. Ordinary lookup/initialization stays fenced.
        let UnitState::Installed(unit) = core::mem::replace(&mut *state, UnitState::Claimed) else {
            unreachable!("validated installed creation unit");
        };
        Ok(Self {
            slot,
            unit: DetachedDomain::new(unit),
            grant,
        })
    }

    fn restore(self) {
        let mut slot = self.slot.lock();
        assert!(matches!(*slot, UnitState::Claimed), "creation claim replaced");
        *slot = UnitState::Installed(self.unit.into_inner());
    }
}

pub(super) fn prepare<T>(
    slot: &Mutex<UnitState<T>>,
    grant: &mut DmaCreation,
    ready: impl FnOnce(&T) -> bool,
    work: impl FnOnce(&mut T, &mut DmaCreation) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut owner = Preparing::begin(slot, grant, ready)?;
    tests::boundary(Phase::Claim);
    let result = work(owner.unit.value_mut(), owner.grant);
    tests::boundary(Phase::Restore);
    // Ordinary hardware errors restore actual queue/epoch/registry state, never
    // reconstruct it. The enclosing grant still owns private/published rollback.
    owner.restore();
    result
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Claim,
    Allocate,
    Configure,
    Restore,
}

pub(super) fn boundary(phase: Phase) {
    tests::boundary(phase);
}
pub(super) fn test_admission() {
    tests::run();
}
pub(super) mod tests;
