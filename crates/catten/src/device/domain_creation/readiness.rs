//! Availability of the existing unit slot, never a physical completion proof.
use alloc::sync::Weak;
use core::mem::ManuallyDrop;

use super::{
    super::{
        DmaCreation,
        unit_initialization::{
            UnitSlot,
            UnitState,
        },
    },
    Error,
};
use crate::{
    cpu::{
        multiprocessor::spin::mutex::Mutex,
        scheduler::deferred,
    },
    klib::{
        collections::retirement_list::PreparedEntry,
        observer::{
            Observable,
            Observer,
            WaitRegistration,
            WaitSponsor,
            registration::{
                ObserverList,
                RegistrationError,
            },
            waiter_budget,
        },
    },
};

struct Payload {
    list: deferred::WaiterList,
    disposal: PreparedEntry<deferred::WaiterList>,
}

/// Source and admitted disposal backing retain together on abandonment.
pub(in crate::device) struct Source(ManuallyDrop<Payload>);

impl Source {
    pub(in crate::device) fn prepare(platform: bool) -> Result<Self, Error> {
        let disposal = PreparedEntry::try_new().map_err(|_| Error::WaitRegistrationFailed)?;
        let list = ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
            .map_err(|_| Error::WaitRegistrationFailed)?;
        Ok(Self(ManuallyDrop::new(Payload {
            list,
            disposal,
        })))
    }

    pub(in crate::device) fn interest(&self) -> Interest {
        Interest(self.0.list.clone())
    }

    pub(super) fn cancel(self) {
        drop(ManuallyDrop::into_inner(self.0));
    }

    pub(in crate::device) fn finish(self) {
        let payload = ManuallyDrop::into_inner(self.0);
        deferred::publish(payload.list.close(), Some(payload.disposal.publish(payload.list)));
    }

    pub(in crate::device) fn registered(&self) -> usize {
        self.0.list.registered()
    }
}

pub(in crate::device) struct Interest(deferred::WaiterList);

impl Interest {
    pub(in crate::device) fn completed(&self) -> bool {
        self.0.is_closed()
    }
}

impl Observable for Interest {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        if self.completed() {
            return Ok(WaitRegistration::ready());
        }
        match sponsor.register(&self.0, observer) {
            Err(RegistrationError::Closed) => Ok(WaitRegistration::ready()),
            other => other,
        }
    }
}

/// The containing transaction supplies captured classification and retains
/// transient interest. No scalar ID or diagnostic creates a wait owner.
pub(in crate::device) trait Owner {
    fn platform(&self) -> Option<bool>;
    fn interest(&mut self) -> &mut Option<Interest>;
    // Diagnostic hooks only, outside backend/lifecycle/scheduler guards.
    fn waiting(&mut self) {}
    fn admitted(&mut self) {}
}
impl Owner for DmaCreation {
    fn platform(&self) -> Option<bool> {
        self.wait_platform
    }

    fn interest(&mut self) -> &mut Option<Interest> {
        &mut self.interest
    }
}

/// Availability admission composes a borrow of the complete operation owner.
/// `claim` runs exactly once under backend serialization, and must return a
/// retaining owner before guard exit. There is no work/cleanup retry here.
pub(in crate::device) fn admit<T, R, O: Owner>(
    slot: &Mutex<UnitSlot<T>>,
    owner: &mut O,
    ready: impl Fn(&T) -> bool,
    claim: impl FnOnce(&mut UnitSlot<T>, &mut O) -> Result<R, Error>,
) -> Result<R, Error> {
    admit_with(
        slot,
        owner,
        ready,
        claim,
        |interest| {
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
            Source::prepare(platform)
        },
    )
}

pub(in crate::device) fn admit_with<T, R, O: Owner>(
    slot: &Mutex<UnitSlot<T>>,
    owner: &mut O,
    ready: impl Fn(&T) -> bool,
    claim: impl FnOnce(&mut UnitSlot<T>, &mut O) -> Result<R, Error>,
    mut wait: impl FnMut(&Interest) -> Result<(), Error>,
    mut prepare: impl FnMut(bool) -> Result<Source, Error>,
) -> Result<R, Error> {
    loop {
        let mut state = slot.lock();
        let available = match &state.state {
            UnitState::Vacant => return Err(Error::Unsupported),
            UnitState::Claimed => false,
            UnitState::Installed(unit) => ready(unit),
        };
        if available {
            let claimed = claim(&mut state, owner);
            drop(state);
            drop(owner.interest().take());
            if claimed.is_ok() {
                owner.admitted();
            }
            return claimed;
        }
        let platform = owner.platform().ok_or(Error::OperationInFlight)?;
        let interest = state.readiness.as_ref().map(Source::interest);
        drop(state);
        if let Some(interest) = interest {
            *owner.interest() = Some(interest);
        } else {
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
            *owner.interest() = Some(interest);
        }
        owner.waiting();
        wait(owner.interest().as_ref().unwrap())?;
        drop(owner.interest().take());
        // A competing claim can win after wake. Recheck this same fixed slot,
        // never reconstruct queues/epochs or replay a callback that ran.
    }
}
