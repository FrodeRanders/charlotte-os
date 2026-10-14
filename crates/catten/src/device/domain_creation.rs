//! Exclusive creation owns the complete installed unit in its existing slot.
//! This composes a borrow of the containing grant; neither fallback can release
//! roots, authority, endpoint claims, tables, queues, mappings or pins.
use super::{
    DmaCreation,
    detached_domain::DetachedDomain,
    dma::Error,
    unit_initialization::{
        UnitSlot,
        UnitState,
    },
};
use crate::cpu::multiprocessor::spin::mutex::Mutex;
pub(super) mod readiness;

#[must_use]
struct Preparing<'slot, 'grant, T> {
    slot: &'slot Mutex<UnitSlot<T>>,
    unit: DetachedDomain<T>,
    grant: &'grant mut DmaCreation,
}

impl<'slot, 'grant, T> Preparing<'slot, 'grant, T> {
    fn begin(
        slot: &'slot Mutex<UnitSlot<T>>,
        grant: &'grant mut DmaCreation,
        ready: impl Fn(&T) -> bool,
    ) -> Result<Self, Error> {
        if grant.is_armed() {
            return Err(Error::OperationInFlight);
        }
        let unit = readiness::admit(slot, grant, ready, |slot, _| {
            let UnitState::Installed(unit) =
                core::mem::replace(&mut slot.state, UnitState::Claimed)
            else {
                unreachable!("validated installed creation unit");
            };
            Ok(DetachedDomain::new(unit))
        })?;
        Ok(Self {
            slot,
            unit,
            grant,
        })
    }

    fn begin_with(
        slot: &'slot Mutex<UnitSlot<T>>,
        grant: &'grant mut DmaCreation,
        ready: impl Fn(&T) -> bool,
        wait: impl FnMut(&readiness::Interest) -> Result<(), Error>,
        prepare: impl FnMut(bool) -> Result<readiness::Source, Error>,
    ) -> Result<Self, Error> {
        if grant.is_armed() {
            return Err(Error::OperationInFlight);
        }
        let unit = readiness::admit_with(
            slot,
            grant,
            ready,
            |slot, _| {
                let UnitState::Installed(unit) =
                    core::mem::replace(&mut slot.state, UnitState::Claimed)
                else {
                    unreachable!("validated installed creation unit");
                };
                Ok(DetachedDomain::new(unit))
            },
            wait,
            prepare,
        )?;
        Ok(Self {
            slot,
            unit,
            grant,
        })
    }

    fn restore(self, ready: impl FnOnce(&T) -> bool) {
        let mut slot = self.slot.lock();
        assert!(matches!(slot.state, UnitState::Claimed), "creation claim replaced");
        slot.state = UnitState::Installed(self.unit.into_inner());
        slot.signal_if(ready);
    }
}

pub(super) fn prepare<T>(
    slot: &Mutex<UnitSlot<T>>,
    grant: &mut DmaCreation,
    ready: impl Fn(&T) -> bool,
    work: impl FnOnce(&mut T, &mut DmaCreation) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut owner = Preparing::begin(slot, grant, &ready)?;
    tests::boundary(Phase::Claim);
    let result = work(owner.unit.value_mut(), owner.grant);
    tests::boundary(Phase::Restore);
    // Ordinary hardware errors restore actual queue/epoch/registry state, never
    // reconstruct it. The enclosing grant still owns private/published rollback.
    owner.restore(ready);
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
