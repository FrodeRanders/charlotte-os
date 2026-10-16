//! Owned, bounded thread-exit subscriptions, independent of the thread table.

use alloc::sync::Weak;
use core::mem::ManuallyDrop;

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

/// Complete transient ownership precedes fallible preparation. Abandonment
/// retains every field; ordinary outcomes finish outside the thread table.
#[must_use]
pub(crate) struct PreparingExitWatch(ManuallyDrop<Resources>);

struct Resources {
    list: Option<ExitList>,
    observer: Option<Weak<dyn Observer>>,
    charge: Option<watch_budget::Charge>,
    entry: Option<PreparedRegistration<watch_budget::Charge>>,
}

impl PreparingExitWatch {
    pub(crate) fn capture(
        existing: Option<ExitList>,
        observer: Weak<dyn Observer>,
        charge: watch_budget::Charge,
    ) -> Self {
        Self(ManuallyDrop::new(Resources {
            list: existing,
            observer: Some(observer),
            charge: Some(charge),
            entry: None,
        }))
    }

    pub(crate) fn new(
        existing: Option<ExitList>,
        observer: Weak<dyn Observer>,
        charge: watch_budget::Charge,
    ) -> Result<Self, RegistrationError> {
        let mut preparation = Self::capture(existing, observer, charge);
        if let Err(error) = preparation.prepare() {
            preparation.finish();
            return Err(error);
        }
        Ok(preparation)
    }

    pub(crate) fn prepare(&mut self) -> Result<(), RegistrationError> {
        self.prepare_with(
            |platform| ObserverList::try_new(watch_budget::MAX_THREAD_WATCHES, platform),
            Self::prepare_entry,
        )
    }

    fn prepare_with(
        &mut self,
        allocate_list: impl FnOnce(bool) -> Result<ExitList, RegistrationError>,
        prepare_entry: impl FnOnce(&mut Self) -> Result<(), RegistrationError>,
    ) -> Result<(), RegistrationError> {
        assert!(self.0.observer.is_some() && self.0.charge.is_some());
        assert!(self.0.entry.is_none());
        if self.0.list.is_none() {
            self.0.list = Some(allocate_list(self.0.charge.as_ref().unwrap().platform())?);
        }
        prepare_entry(self)?;
        assert!(self.0.entry.is_some(), "exit-watch factory returned no entry");
        assert!(self.0.observer.is_none() && self.0.charge.is_none());
        Ok(())
    }

    fn prepare_entry(&mut self) -> Result<(), RegistrationError> {
        assert!(self.0.list.is_some() && self.0.entry.is_none());
        // The shared constructor owns both arguments before empty allocation;
        // this containing owner retains the exact candidate source throughout.
        self.0.entry = Some(PreparedRegistration::try_new(
            self.0.observer.take().unwrap(),
            self.0.charge.take().unwrap(),
        )?);
        Ok(())
    }

    pub(crate) fn finish(self) {
        self.finish_with(|| {});
    }

    fn finish_with(mut self, mut before_dispose: impl FnMut()) {
        before_dispose();
        drop(self.0.observer.take());
        before_dispose();
        drop(self.0.entry.take());
        before_dispose();
        drop(self.0.charge.take());
        // Keep source identity/backing through entry and admission disposal.
        before_dispose();
        drop(self.0.list.take());
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
        let result = self.register_prepared(&mut preparation);
        preparation.finish();
        result
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
                *slot = Some(preparation.0.list.as_ref().unwrap().clone());
            }
            slot.as_ref().unwrap().clone()
        };
        list.register_prepared(preparation.0.entry.as_mut().unwrap())
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

mod preparation_tests;

pub(crate) fn test_preparation() {
    preparation_tests::run();
}
