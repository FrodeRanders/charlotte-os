//! Owned, bounded thread-exit subscriptions, independent of the thread table.

use alloc::sync::Weak;

use crate::{
    completion::watch_budget,
    cpu::multiprocessor::spin::mutex::Mutex,
    klib::observer::{
        Observer,
        registration::{
            ListRef,
            ObserverList,
            Registration,
            RegistrationError,
        },
    },
};

pub(crate) type ExitRegistration = Registration<watch_budget::Charge>;

#[derive(Debug)]
pub(crate) struct ExitSource {
    list: Mutex<Option<ListRef<watch_budget::Charge>>>,
}

impl ExitSource {
    pub(crate) const fn new() -> Self {
        Self {
            list: Mutex::new(None),
        }
    }

    pub(crate) fn register(
        &self,
        observer: Weak<dyn Observer>,
        charge: watch_budget::Charge,
    ) -> Result<ExitRegistration, RegistrationError> {
        let list = {
            let mut slot = self.list.lock();
            if slot.is_none() {
                *slot = Some(ObserverList::try_new(
                    watch_budget::MAX_THREAD_WATCHES,
                    charge.platform(),
                )?);
            }
            slot.as_ref().unwrap().clone()
        };
        list.register(observer, charge)
    }

    pub(crate) fn registered(&self) -> usize {
        self.list.lock().as_ref().map_or(0, |list| list.registered())
    }

    /// Called exactly once by Thread::drop, before its stack is deallocated.
    /// No source/list or scheduler table guard is held during notification.
    pub(crate) fn notify_exit(&mut self) {
        if let Some(list) = self.list.get_mut().take() {
            list.close().notify();
        }
    }
}

impl Drop for ExitSource {
    fn drop(&mut self) {
        if let Some(list) = self.list.get_mut().take() {
            // An unpublished/discarded source must not fabricate an exit.
            drop(list.close());
        }
    }
}
