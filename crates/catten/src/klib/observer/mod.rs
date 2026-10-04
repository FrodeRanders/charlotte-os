//! Observer pattern implementation for event notification

pub(crate) mod registration;
pub(crate) mod waiter_budget;
pub(crate) mod waiter_source;

use alloc::sync::{
    Arc,
    Weak,
};

pub trait Observable {
    fn register_observer(&self, observer: Weak<dyn Observer>);

    /// Never invoke callbacks inline: the scheduler holds its thread table.
    /// Converted sources return an owning, fallible registration. Legacy
    /// sources retain their old unbounded storage and return a marked token.
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        _sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, registration::RegistrationError> {
        self.register_observer(observer);
        Ok(WaitRegistration::legacy())
    }
}

#[derive(Debug, Clone)]
pub struct WaitSponsor(Arc<waiter_budget::DomainBudget>);
impl WaitSponsor {
    pub(crate) fn new(platform: bool) -> Self {
        Self(waiter_budget::DomainBudget::new(platform))
    }

    pub(crate) fn retire(&self) {
        self.0.retire();
    }

    pub(crate) fn mark_platform(&self) {
        self.0.mark_platform();
    }

    pub(crate) fn used(&self) -> usize {
        self.0.used()
    }

    pub(crate) fn register(
        &self,
        list: &Arc<registration::ObserverList<waiter_budget::Charge>>,
        observer: Weak<dyn Observer>,
    ) -> Result<WaitRegistration, registration::RegistrationError> {
        let charge = waiter_budget::reserve(&self.0)?;
        Ok(WaitRegistration {
            owned: Some(list.register(observer, charge)?),
            ready: false,
        })
    }
}

#[must_use = "retain an owning waiter registration until wake or cancellation"]
#[derive(Debug)]
pub struct WaitRegistration {
    owned: Option<registration::Registration<waiter_budget::Charge>>,
    ready: bool,
}
impl WaitRegistration {
    pub(crate) const fn legacy() -> Self {
        Self {
            owned: None,
            ready: false,
        }
    }

    pub(crate) const fn ready() -> Self {
        Self {
            owned: None,
            ready: true,
        }
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.ready
    }

    pub(crate) fn is_owned(&self) -> bool {
        self.owned.is_some()
    }
}

/// An `Observer` is an object that can be notified of events by an `Observable`.
/// Observers must be `Sync` because they may be notified from multiple threads concurrently.
pub trait Observer: Send + Sync {
    /// Called by an Observable when it wants to notify this Observer of an event.
    fn notify(self: Arc<Self>);
}

/// A generic `Observer` that calls a function object when it is notified.
/// This can be used to create observers that execute arbitrary code when notified without needing
/// to create a new struct for each one.
///
/// Note: Do not use this with very long callbacks as it is called into from an Observable's
/// notification loop which may need to notify many observers and thus should be as efficient as
/// possible. If a large amount of work needs to be done in response to an event then the callback
/// should spawn a proper kernel thread to do the work instead.
pub struct CallOnNotify<F: Fn() + Send + Sync> {
    callback: F,
}

impl<F: Fn() + Send + Sync> CallOnNotify<F> {
    pub fn new(callback: F) -> Arc<Self> {
        Arc::new(CallOnNotify {
            callback,
        })
    }
}

impl<F: Fn() + Send + Sync> Observer for CallOnNotify<F> {
    #[inline(always)]
    fn notify(self: Arc<Self>) {
        (self.callback)();
    }
}
