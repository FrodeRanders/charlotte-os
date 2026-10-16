//! Complete transient ownership across lazy source selection and waiter handoff.

use core::mem::ManuallyDrop;

use super::*;
use crate::klib::observer::PreparedWaitRegistration;

type List = ListRef<waiter_budget::Charge>;

#[must_use]
pub(super) struct Preparing(ManuallyDrop<Resources>);

struct Resources {
    sponsor: Option<WaitSponsor>,
    observer: Option<Weak<dyn Observer>>,
    candidate: Option<List>,
    selected: Option<List>,
    entry: Option<PreparedWaitRegistration>,
}

impl Preparing {
    pub(super) fn capture(sponsor: &WaitSponsor, observer: Weak<dyn Observer>) -> Self {
        Self(ManuallyDrop::new(Resources {
            sponsor: Some(sponsor.clone()),
            observer: Some(observer),
            candidate: None,
            selected: None,
            entry: None,
        }))
    }

    pub(super) fn prepare_list(
        &mut self,
        source: &WaiterSource,
        allocate: impl FnOnce(bool) -> Result<List, RegistrationError>,
    ) -> Result<(), RegistrationError> {
        assert!(self.0.candidate.is_none() && self.0.selected.is_none());
        let existing = source.list.lock().clone();
        self.0.candidate = Some(match existing {
            Some(list) => list,
            None => allocate(self.0.sponsor.as_ref().unwrap().list_platform()?)?,
        });
        Ok(())
    }

    pub(super) fn candidate(&self) -> &List {
        self.0.candidate.as_ref().unwrap()
    }

    pub(super) fn selected(&self) -> &List {
        self.0.selected.as_ref().unwrap()
    }

    pub(super) fn select(&mut self, source: &WaiterSource) {
        assert!(self.0.selected.is_none());
        self.0.selected = Some(source.publish_list(self.candidate()));
    }

    pub(super) fn release_candidate(&mut self) {
        self.release_candidate_with(|| {});
    }

    fn release_candidate_with(&mut self, before_dispose: impl FnOnce()) {
        assert!(self.0.selected.is_some() && self.0.entry.is_none());
        before_dispose();
        drop(self.0.candidate.take());
    }

    pub(super) fn prepare_entry(&mut self) -> Result<(), RegistrationError> {
        assert!(self.0.candidate.is_none() && self.0.entry.is_none());
        let resources = &mut *self.0;
        // The shared child captures these exact source/sponsor references and
        // consumes the weak callback before admission or empty-entry allocation.
        resources.entry =
            Some(resources.sponsor.as_ref().unwrap().prepare(
                resources.selected.as_ref().unwrap(),
                resources.observer.take().unwrap(),
            )?);
        Ok(())
    }

    pub(super) fn publish_entry(&mut self) -> Result<WaitRegistration, RegistrationError> {
        self.0.entry.as_mut().unwrap().publish()
    }

    pub(super) fn finish(self) {
        self.finish_with(|| {});
    }

    fn finish_with(mut self, mut before_dispose: impl FnMut()) {
        before_dispose();
        drop(self.0.observer.take());
        before_dispose();
        if let Some(entry) = self.0.entry.take() {
            entry.finish();
        }
        before_dispose();
        drop(self.0.candidate.take());
        before_dispose();
        drop(self.0.selected.take());
        before_dispose();
        drop(self.0.sponsor.take());
    }
}

mod tests;

pub(super) fn test_preparation() {
    tests::run();
}
