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
            PreparedRegistration,
            Registration,
            RegistrationError,
        },
    },
};

pub(crate) type ExitRegistration = Registration<watch_budget::Charge>;
pub(crate) type ExitList = ListRef<watch_budget::Charge>;

/// Retains both possible source storage and the charged entry across exact
/// thread revalidation. Losing storage is disposed outside the thread table.
pub(crate) struct PreparingExitWatch {
    list: ExitList,
    entry: PreparedRegistration<watch_budget::Charge>,
}

impl PreparingExitWatch {
    pub(crate) fn new(
        existing: Option<ExitList>,
        observer: Weak<dyn Observer>,
        charge: watch_budget::Charge,
    ) -> Result<Self, RegistrationError> {
        let list = match existing {
            Some(list) => list,
            None => ObserverList::try_new(watch_budget::MAX_THREAD_WATCHES, charge.platform())?,
        };
        let entry = PreparedRegistration::try_new(observer, charge)?;
        Ok(Self {
            list,
            entry,
        })
    }
}

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
        let mut preparation = PreparingExitWatch::new(self.source(), observer, charge)?;
        self.register_prepared(&mut preparation)
    }

    pub(crate) fn source(&self) -> Option<ExitList> {
        self.list.lock().clone()
    }

    pub(crate) fn register_prepared(
        &self,
        preparation: &mut PreparingExitWatch,
    ) -> Result<ExitRegistration, RegistrationError> {
        let list = {
            let mut slot = self.list.lock();
            if slot.is_none() {
                *slot = Some(preparation.list.clone());
            }
            slot.as_ref().unwrap().clone()
        };
        list.register_prepared(&mut preparation.entry)
    }

    pub(crate) fn registered(&self) -> usize {
        self.list.lock().as_ref().map_or(0, |list| list.registered())
    }

    /// Called exactly once by explicit thread retirement, before stack release.
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
