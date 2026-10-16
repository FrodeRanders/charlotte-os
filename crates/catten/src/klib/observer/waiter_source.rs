//! Const-initializable owning waiter source for kernel blocking locks and
//! timer events. The list is allocated fallibly on first registration;
//! uncontended locks allocate nothing. Tokens enter only the independent list
//! lock on cancellation.

use alloc::sync::Weak;

use preparation::Preparing;

use super::{
    Observable,
    Observer,
    WaitRegistration,
    WaitSponsor,
    registration::{
        ListRef,
        NotificationBatch,
        ObserverList,
        RegistrationError,
    },
    waiter_budget,
};
use crate::cpu::multiprocessor::spin::mutex::Mutex;

#[derive(Debug)]
pub(crate) struct WaiterSource {
    list: Mutex<Option<ListRef<waiter_budget::Charge>>>,
}

impl WaiterSource {
    pub(crate) const fn new() -> Self {
        Self {
            list: Mutex::new(None),
        }
    }

    pub(crate) fn register(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        self.register_with(observer, sponsor, |platform| {
            ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
        })
    }

    fn register_with(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
        allocate: impl FnOnce(bool) -> Result<ListRef<waiter_budget::Charge>, RegistrationError>,
    ) -> Result<WaitRegistration, RegistrationError> {
        let mut preparing = Preparing::capture(sponsor, observer);
        let result = preparing.prepare_list(self, allocate).and_then(|()| {
            preparing.select(self);
            // Release losing storage after the slot guard, before entry
            // admission, while retaining winner/sponsor/callback ownership.
            preparing.release_candidate();
            preparing.prepare_entry()?;
            preparing.publish_entry()
        });
        preparing.finish();
        result
    }

    fn publish_list(
        &self,
        candidate: &ListRef<waiter_budget::Charge>,
    ) -> ListRef<waiter_budget::Charge> {
        let mut slot = self.list.lock();
        if slot.is_none() {
            *slot = Some(candidate.clone());
        }
        slot.as_ref().unwrap().clone()
    }

    pub(crate) fn registered(&self) -> usize {
        self.list.lock().as_ref().map_or(0, |list| list.registered())
    }

    /// Detach under independent source/list locks. Callbacks must run only
    /// after those guards and the blocking lock's data ownership are released.
    pub(crate) fn drain(&self) -> NotificationBatch<waiter_budget::Charge> {
        let list = self.list.lock().as_ref().cloned();
        list.map_or_else(NotificationBatch::empty, |list| list.drain())
    }
}

impl Default for WaiterSource {
    fn default() -> Self {
        Self::new()
    }
}

impl Observable for WaiterSource {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        self.register(observer, sponsor)
    }
}

impl Drop for WaiterSource {
    fn drop(&mut self) {
        if let Some(list) = self.list.get_mut() {
            // Retained registration tokens must not retain source entries.
            drop(list.close());
        }
    }
}

mod preparation;
mod tests;

pub(crate) fn test_preparation() {
    tests::run();
}

pub(crate) fn test_owner_preparation() {
    preparation::test_preparation();
}
