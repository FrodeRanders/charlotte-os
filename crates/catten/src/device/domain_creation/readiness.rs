//! Availability of the existing unit slot, never a physical completion proof.
use alloc::sync::Weak;
use core::mem::ManuallyDrop;

use super::Error;
use crate::{
    cpu::scheduler::deferred,
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
    pub(super) fn completed(&self) -> bool {
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
