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
    pub(super) fn prepare(platform: bool) -> Result<Self, Error> {
        let disposal = PreparedEntry::try_new().map_err(|_| Error::WaitRegistrationFailed)?;
        let list = ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
            .map_err(|_| Error::WaitRegistrationFailed)?;
        Ok(Self(ManuallyDrop::new(Payload {
            list,
            disposal,
        })))
    }

    pub(super) fn interest(&self) -> Interest {
        Interest(self.0.list.clone())
    }

    pub(super) fn cancel(self) {
        drop(ManuallyDrop::into_inner(self.0));
    }

    pub(in crate::device) fn finish(self) {
        let payload = ManuallyDrop::into_inner(self.0);
        deferred::publish(payload.list.close(), Some(payload.disposal.publish(payload.list)));
    }

    pub(super) fn registered(&self) -> usize {
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

/// Availability admission composes a borrow of the complete operation owner.
/// `claim` runs exactly once under backend serialization, and must return a
/// retaining owner before guard exit. There is no work/cleanup retry here.
pub(in crate::device) fn admit<T, R>(
    slot: &Mutex<UnitSlot<T>>,
    grant: &mut DmaCreation,
    ready: impl Fn(&T) -> bool,
    claim: impl FnOnce(&mut UnitSlot<T>) -> Result<R, Error>,
) -> Result<R, Error> {
    admit_with(
        slot,
        grant,
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

pub(super) fn admit_with<T, R>(
    slot: &Mutex<UnitSlot<T>>,
    grant: &mut DmaCreation,
    ready: impl Fn(&T) -> bool,
    claim: impl FnOnce(&mut UnitSlot<T>) -> Result<R, Error>,
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
            let owner = claim(&mut state);
            drop(state);
            drop(grant.interest.take());
            return owner;
        }
        let platform = grant.wait_platform.ok_or(Error::OperationInFlight)?;
        let interest = state.readiness.as_ref().map(Source::interest);
        drop(state);
        if let Some(interest) = interest {
            *grant.interest = Some(interest);
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
            *grant.interest = Some(interest);
        }
        wait(grant.interest.as_ref().unwrap())?;
        drop(grant.interest.take());
        // A competing claim can win after wake. Recheck this same fixed slot,
        // never reconstruct queues/epochs or replay a callback that ran.
    }
}
