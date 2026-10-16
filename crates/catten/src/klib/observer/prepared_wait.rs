//! Exact waiter source/sponsor ownership before admission and entry allocation.

use core::mem::ManuallyDrop;

use registration::{
    ListRef,
    PreparedRegistration,
    RegistrationError,
};

use super::*;

type Source = ListRef<waiter_budget::Charge>;

/// Publication borrows the complete owner. Abandonment retains every partial
/// field; ordinary outcomes explicitly finish after local source guards leave.
#[must_use]
pub(crate) struct PreparedWaitRegistration(ManuallyDrop<Resources>);

struct Resources {
    source: Option<Source>,
    sponsor: Option<WaitSponsor>,
    observer: Option<Weak<dyn Observer>>,
    charge: Option<waiter_budget::Charge>,
    entry: Option<PreparedRegistration<waiter_budget::Charge>>,
}

impl PreparedWaitRegistration {
    fn capture(sponsor: &WaitSponsor, source: &Source, observer: Weak<dyn Observer>) -> Self {
        Self(ManuallyDrop::new(Resources {
            source: Some(source.clone()),
            sponsor: Some(sponsor.clone()),
            observer: Some(observer),
            charge: None,
            entry: None,
        }))
    }

    pub(super) fn new(
        sponsor: &WaitSponsor,
        source: &Source,
        observer: Weak<dyn Observer>,
    ) -> Result<Self, RegistrationError> {
        Self::new_with(sponsor, source, observer, Self::entry)
    }

    fn new_with(
        sponsor: &WaitSponsor,
        source: &Source,
        observer: Weak<dyn Observer>,
        prepare_entry: impl FnOnce(&mut Self) -> Result<(), RegistrationError>,
    ) -> Result<Self, RegistrationError> {
        let mut prepared = Self::capture(sponsor, source, observer);
        let result = prepared.admit().and_then(|()| prepare_entry(&mut prepared)).map(|()| {
            assert!(prepared.0.entry.is_some(), "waiter factory returned no entry");
            assert!(prepared.0.observer.is_none() && prepared.0.charge.is_none());
        });
        if let Err(error) = result {
            prepared.finish();
            return Err(error);
        }
        Ok(prepared)
    }

    fn admit(&mut self) -> Result<(), RegistrationError> {
        assert!(self.0.charge.is_none() && self.0.entry.is_none());
        self.0.charge = Some(waiter_budget::reserve(&self.0.sponsor.as_ref().unwrap().0)?);
        Ok(())
    }

    fn entry(&mut self) -> Result<(), RegistrationError> {
        assert!(self.0.entry.is_none());
        // Transfer directly into the shared complete entry constructor. The
        // exact source and original sponsor remain with this containing owner.
        self.0.entry = Some(PreparedRegistration::try_new(
            self.0.observer.take().unwrap(),
            self.0.charge.take().unwrap(),
        )?);
        Ok(())
    }

    pub(crate) fn matches(&self, source: &Source) -> bool {
        core::ptr::eq(&**self.0.source.as_ref().unwrap(), &**source)
    }

    pub(crate) fn publish(&mut self) -> Result<WaitRegistration, RegistrationError> {
        let resources = &mut *self.0;
        Ok(WaitRegistration {
            owned: Some(
                resources
                    .source
                    .as_ref()
                    .unwrap()
                    .register_prepared(resources.entry.as_mut().unwrap())?,
            ),
            ready: false,
        })
    }

    pub(crate) fn finish(self) {
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
        drop(self.0.charge.take());
        before_dispose();
        drop(self.0.source.take());
        // Keep the original sponsor through all transient/source disposal.
        before_dispose();
        drop(self.0.sponsor.take());
    }
}

mod tests;

pub(super) fn test_preparation() {
    tests::run();
}
