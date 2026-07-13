//! Observer pattern implementation for event notification

use alloc::sync::{
    Arc,
    Weak,
};

pub trait Observable {
    fn register_observer(&self, observer: Weak<dyn Observer>);
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
