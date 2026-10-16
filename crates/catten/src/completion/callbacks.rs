//! Captured-operation callback preparation outside registry/inner guards.

use super::*;
use crate::klib::observer::registration::{
    ListRef,
    ObserverList,
    PreparedRegistration,
    Registration,
    RegistrationError,
};

type CallbackList = ListRef<watch_budget::Charge>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveError {
    UnknownAddressSpace,
    UnknownCap,
    ResourceLimit,
}

/// Cancels only the subscription, never its producer or borrowed resources.
/// A callback already captured for notification may race cancellation.
#[must_use = "retain the callback owner until notification or cancellation"]
pub struct CompletionObservation {
    _registration: Option<Registration<watch_budget::Charge>>,
    _observer: Arc<dyn Observer>,
}

impl core::fmt::Debug for CompletionObservation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CompletionObservation")
            .field("has_token", &self._registration.is_some())
            .finish_non_exhaustive()
    }
}

struct Captured {
    asid: AddressSpaceId,
    cap: CompletionCap,
    completion: CompletionRef,
    root: Option<crate::memory::AddressSpaceHandle>,
    account: Arc<watch_budget::DomainBudget>,
    platform: bool,
    terminal: bool,
    list: Option<CallbackList>,
}

impl Captured {
    fn new(
        asid: AddressSpaceId,
        cap: CompletionCap,
        captured: &CompletionRef,
    ) -> Result<Self, ObserveError> {
        let platform_identity = crate::memory::budget::platform_identity(asid);
        let registry = COMPLETIONS.read();
        let entries = registry.get(&asid).ok_or(ObserveError::UnknownAddressSpace)?;
        if entries.address_space.is_some_and(|handle| !crate::memory::budget::accepting(handle)) {
            return Err(ObserveError::UnknownAddressSpace);
        }
        let completion = entries.table.get(&cap).ok_or(ObserveError::UnknownCap)?;
        if !Arc::ptr_eq(completion, captured) {
            return Err(ObserveError::UnknownCap);
        }
        let inner = completion.inner.lock();
        Ok(Self {
            asid,
            cap,
            completion: Arc::clone(completion),
            root: entries.address_space,
            account: entries.watch_budget.clone(),
            platform: asid == crate::memory::KERNEL_ASID
                || platform_identity.is_some_and(|handle| Some(handle) == entries.address_space),
            terminal: matches!(inner.state, OpState::Completed(_) | OpState::Observed),
            list: inner.callbacks.clone(),
        })
    }
}

/// Complete transient ownership survives construction and borrowed publication.
/// Abandonment retains all fields; ordinary outcomes finish after local guards.
#[must_use]
struct Preparing(ManuallyDrop<Resources>);

struct Resources {
    target: Option<Captured>,
    observer: Option<Weak<dyn Observer>>,
    charge: Option<watch_budget::Charge>,
    candidate: Option<CallbackList>,
    entry: Option<PreparedRegistration<watch_budget::Charge>>,
}

impl Preparing {
    fn capture(target: Captured, observer: Weak<dyn Observer>) -> Self {
        Self(ManuallyDrop::new(Resources {
            target: Some(target),
            observer: Some(observer),
            charge: None,
            candidate: None,
            entry: None,
        }))
    }

    fn target(&self) -> &Captured {
        self.0.target.as_ref().unwrap()
    }

    fn admit(&mut self) -> Result<(), ObserveError> {
        let resources = &mut *self.0;
        assert!(resources.charge.is_none() && resources.entry.is_none());
        let target = resources.target.as_ref().unwrap();
        resources.charge = Some(
            watch_budget::reserve(&target.account, target.platform)
                .map_err(|_| ObserveError::ResourceLimit)?,
        );
        Ok(())
    }

    fn candidate_with(
        &mut self,
        allocate: impl FnOnce(bool) -> Result<CallbackList, RegistrationError>,
    ) -> Result<(), ObserveError> {
        assert!(self.0.candidate.is_none() && self.0.charge.is_some());
        self.0.candidate = Some(match &self.target().list {
            Some(list) => list.clone(),
            None => allocate(self.0.charge.as_ref().unwrap().platform())
                .map_err(|_| ObserveError::ResourceLimit)?,
        });
        Ok(())
    }

    fn entry(&mut self) -> Result<(), ObserveError> {
        assert!(self.0.entry.is_none() && self.0.candidate.is_some());
        // Transfer directly into the shared complete-entry constructor. The
        // exact captured record and independent candidate list remain here.
        self.0.entry = Some(
            PreparedRegistration::try_new(
                self.0.observer.take().unwrap(),
                self.0.charge.take().unwrap(),
            )
            .map_err(|_| ObserveError::ResourceLimit)?,
        );
        Ok(())
    }

    fn new(target: Captured, observer: Weak<dyn Observer>) -> Result<Self, ObserveError> {
        Self::new_with(
            target,
            observer,
            |platform| ObserverList::try_new(watch_budget::MAX_COMPLETION_CALLBACKS, platform),
            Self::entry,
        )
    }

    fn new_with(
        target: Captured,
        observer: Weak<dyn Observer>,
        allocate_list: impl FnOnce(bool) -> Result<CallbackList, RegistrationError>,
        prepare_entry: impl FnOnce(&mut Self) -> Result<(), ObserveError>,
    ) -> Result<Self, ObserveError> {
        let mut preparing = Self::capture(target, observer);
        let result = if preparing.target().terminal {
            Ok(())
        } else {
            preparing
                .admit()
                .and_then(|()| preparing.candidate_with(allocate_list))
                .and_then(|()| prepare_entry(&mut preparing))
                .map(|()| {
                    assert!(preparing.0.entry.is_some(), "callback factory returned no entry");
                    assert!(preparing.0.observer.is_none() && preparing.0.charge.is_none());
                })
        };
        if let Err(error) = result {
            preparing.finish();
            return Err(error);
        }
        Ok(preparing)
    }

    fn publish(&mut self) -> Result<Option<Registration<watch_budget::Charge>>, ObserveError> {
        let resources = &mut *self.0;
        let target = resources.target.as_ref().unwrap();
        let registry = COMPLETIONS.read();
        let entries = registry.get(&target.asid).ok_or(ObserveError::UnknownAddressSpace)?;
        if entries.address_space != target.root
            || !Arc::ptr_eq(&entries.watch_budget, &target.account)
            || entries.address_space.is_some_and(|handle| !crate::memory::budget::accepting(handle))
        {
            return Err(ObserveError::UnknownAddressSpace);
        }
        let completion = entries.table.get(&target.cap).ok_or(ObserveError::UnknownCap)?;
        if !Arc::ptr_eq(completion, &target.completion) {
            return Err(ObserveError::UnknownCap);
        }
        let mut inner = completion.inner.lock();
        // The terminal check and allocation-free insertion share the same
        // lock as completion. Never append after one-shot delivery has drained.
        if matches!(inner.state, OpState::Completed(_) | OpState::Observed) {
            return Ok(None);
        }
        if let Some(original) = &target.list
            && !inner.callbacks.as_ref().is_some_and(|list| core::ptr::eq(&**list, &**original))
        {
            return Err(ObserveError::UnknownCap);
        }
        let entry = resources.entry.as_mut().ok_or(ObserveError::UnknownCap)?;
        if inner.callbacks.is_none() {
            inner.callbacks = Some(resources.candidate.as_ref().unwrap().clone());
        }
        inner
            .callbacks
            .as_ref()
            .unwrap()
            .register_prepared(entry)
            .map(Some)
            .map_err(|_| ObserveError::ResourceLimit)
    }

    fn finish(self) {
        self.finish_with(|| {});
    }

    fn finish_with(mut self, mut before_dispose: impl FnMut()) {
        before_dispose();
        drop(self.0.observer.take());
        before_dispose();
        drop(self.0.candidate.take());
        before_dispose();
        drop(self.0.entry.take());
        before_dispose();
        drop(self.0.charge.take());
        // Keep the exact record/account/source capture through all transient
        // disposal. This final release is explicit too, outside local guards.
        before_dispose();
        drop(self.0.target.take());
    }
}

/// Capture a namespace's current completion, then use exact-object admission.
pub fn observe(
    asid: AddressSpaceId,
    cap: CompletionCap,
    observer: Arc<dyn Observer>,
) -> Result<CompletionObservation, ObserveError> {
    let completion = {
        let registry = COMPLETIONS.read();
        let entries = registry.get(&asid).ok_or(ObserveError::UnknownAddressSpace)?;
        Arc::clone(entries.table.get(&cap).ok_or(ObserveError::UnknownCap)?)
    };
    observe_registered(asid, cap, &completion, observer)
}

/// Previously captured operations reject namespace/handle replacement. Ready
/// callbacks run once after losing storage and all local guards have left.
pub(crate) fn observe_registered(
    asid: AddressSpaceId,
    cap: CompletionCap,
    captured: &CompletionRef,
    observer: Arc<dyn Observer>,
) -> Result<CompletionObservation, ObserveError> {
    let target = Captured::new(asid, cap, captured)?;
    let mut prepared = Preparing::new(target, Arc::downgrade(&observer))?;
    let registration = prepared.publish();
    prepared.finish();
    let registration = registration?;
    let ready = registration.is_none();
    let observation = CompletionObservation {
        _registration: registration,
        _observer: observer,
    };
    if ready {
        observation._observer.clone().notify();
    }
    Ok(observation)
}

mod preparation_tests;
mod tests;

pub(super) fn test_preparation() {
    tests::run();
    preparation_tests::run();
}
