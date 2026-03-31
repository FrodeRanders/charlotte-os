//! Observer pattern implementation for event handling and notifications.

use alloc::sync::Weak;

pub mod combinators;

pub trait Observable {
    fn register_observer(&mut self, observer: Weak<dyn Observer>);
}

pub trait Observer: Send + Sync {
    fn notify(&self);
}
