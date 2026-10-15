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
            completion: completion.clone(),
            root: entries.address_space,
            account: entries.watch_budget.clone(),
            platform: asid == crate::memory::KERNEL_ASID
                || platform_identity.is_some_and(|handle| Some(handle) == entries.address_space),
            terminal: matches!(inner.state, OpState::Completed(_) | OpState::Observed),
            list: inner.callbacks.clone(),
        })
    }
}

struct Storage {
    entry: PreparedRegistration<watch_budget::Charge>,
    candidate: CallbackList,
}

/// All transient list/entry/account and exact-record ownership remains here
/// through borrowed publication, including ready, losing and error outcomes.
struct Preparing {
    target: Captured,
    storage: Option<Storage>,
}

impl Preparing {
    fn new(target: Captured, observer: Weak<dyn Observer>) -> Result<Self, ObserveError> {
        Self::new_with(target, observer, |platform| {
            ObserverList::try_new(watch_budget::MAX_COMPLETION_CALLBACKS, platform)
        })
    }

    fn new_with(
        target: Captured,
        observer: Weak<dyn Observer>,
        allocate_list: impl FnOnce(bool) -> Result<CallbackList, RegistrationError>,
    ) -> Result<Self, ObserveError> {
        let storage = if target.terminal {
            None
        } else {
            let charge = watch_budget::reserve(&target.account, target.platform)
                .map_err(|_| ObserveError::ResourceLimit)?;
            let candidate = match &target.list {
                Some(list) => list.clone(),
                None => {
                    allocate_list(charge.platform()).map_err(|_| ObserveError::ResourceLimit)?
                }
            };
            let entry = PreparedRegistration::try_new(observer, charge)
                .map_err(|_| ObserveError::ResourceLimit)?;
            Some(Storage {
                entry,
                candidate,
            })
        };
        Ok(Self {
            target,
            storage,
        })
    }

    fn publish(&mut self) -> Result<Option<Registration<watch_budget::Charge>>, ObserveError> {
        let registry = COMPLETIONS.read();
        let entries = registry.get(&self.target.asid).ok_or(ObserveError::UnknownAddressSpace)?;
        if entries.address_space != self.target.root
            || !Arc::ptr_eq(&entries.watch_budget, &self.target.account)
            || entries.address_space.is_some_and(|handle| !crate::memory::budget::accepting(handle))
        {
            return Err(ObserveError::UnknownAddressSpace);
        }
        let completion = entries.table.get(&self.target.cap).ok_or(ObserveError::UnknownCap)?;
        if !Arc::ptr_eq(completion, &self.target.completion) {
            return Err(ObserveError::UnknownCap);
        }
        let mut inner = completion.inner.lock();
        // The terminal check and allocation-free insertion share the same
        // lock as completion. Never append after one-shot delivery has drained.
        if matches!(inner.state, OpState::Completed(_) | OpState::Observed) {
            return Ok(None);
        }
        if let Some(original) = &self.target.list
            && !inner.callbacks.as_ref().is_some_and(|list| core::ptr::eq(&**list, &**original))
        {
            return Err(ObserveError::UnknownCap);
        }
        let storage = self.storage.as_mut().ok_or(ObserveError::UnknownCap)?;
        if inner.callbacks.is_none() {
            inner.callbacks = Some(storage.candidate.clone());
        }
        inner
            .callbacks
            .as_ref()
            .unwrap()
            .register_prepared(&mut storage.entry)
            .map(Some)
            .map_err(|_| ObserveError::ResourceLimit)
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
        entries.table.get(&cap).ok_or(ObserveError::UnknownCap)?.clone()
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
    let registration = prepared.publish()?;
    drop(prepared);
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

mod tests;

pub(super) fn test_preparation() {
    tests::run();
}
