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
        Self::begin_with(
            slot,
            grant,
            ready,
            |interest| {
                // Never restore/enable IRQs beneath an unknown caller mask.
                if !crate::cpu::isa::lp::ops::get_int_state() {
                    return Err(Error::WaitRegistrationFailed);
                }
                crate::cpu::scheduler::try_park_until(
                    interest,
                    crate::cpu::scheduler::threads::MigrationConstraint::GeneralWait,
                    || interest.completed(),
                )
                .map_err(|_| Error::WaitRegistrationFailed)
            },
            |platform| {
                if !crate::cpu::isa::lp::ops::get_int_state()
                    || crate::cpu::scheduler::system_scheduler::get_thread_id().is_none()
                {
                    return Err(Error::WaitRegistrationFailed);
                }
                readiness::Source::prepare(platform)
            },
        )
    }

    fn begin_with(
        slot: &'slot Mutex<UnitSlot<T>>,
        grant: &'grant mut DmaCreation,
        ready: impl Fn(&T) -> bool,
        mut wait: impl FnMut(&readiness::Interest) -> Result<(), Error>,
        mut prepare: impl FnMut(bool) -> Result<readiness::Source, Error>,
    ) -> Result<Self, Error> {
        if grant.is_armed() {
            return Err(Error::OperationInFlight);
        }
        loop {
            let mut state = slot.lock();
            let available = match &state.state {
                UnitState::Vacant => return Err(Error::Unsupported),
                UnitState::Claimed => false,
                UnitState::Installed(unit) => ready(unit),
            };
            if available {
                // No allocation or fallible work between extraction and the
                // complete retaining owner. Work is run once, after admission.
                let UnitState::Installed(unit) =
                    core::mem::replace(&mut state.state, UnitState::Claimed)
                else {
                    unreachable!("validated installed creation unit");
                };
                let unit = DetachedDomain::new(unit);
                drop(state);
                drop(grant.interest.take());
                return Ok(Self {
                    slot,
                    unit,
                    grant,
                });
            }
            let platform = grant.wait_platform.ok_or(Error::OperationInFlight)?;
            let interest = state.readiness.as_ref().map(readiness::Source::interest);
            drop(state);
            if let Some(interest) = interest {
                *grant.interest = Some(interest);
            } else {
                // Both allocations precede backend serialization. A producer
                // or another waiter may win before exact slot revalidation.
                let mut prepared = Some(prepare(platform)?);
                let mut state = slot.lock();
                let busy = match &state.state {
                    UnitState::Claimed => true,
                    UnitState::Installed(unit) => !ready(unit),
                    UnitState::Vacant => false,
                };
                let interest = if busy {
                    if state.readiness.is_none() {
                        state.readiness = prepared.take();
                    }
                    Some(state.readiness.as_ref().unwrap().interest())
                } else {
                    None
                };
                drop(state);
                if let Some(unused) = prepared {
                    unused.cancel();
                }
                let Some(interest) = interest else {
                    continue;
                };
                *grant.interest = Some(interest);
            }
            wait(grant.interest.as_ref().unwrap())?;
            drop(grant.interest.take());
            // A notification is availability, not ownership. Recheck the
            // current slot; another operation may have claimed it first.
        }
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
