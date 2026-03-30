//! Observer pattern implementation for event handling and notifications.

pub mod combinators;

pub trait Observable<'a> {
    fn register_observer(&'a self, observer: &'a dyn Observer);
}

pub trait Observer {
    fn notify(&self);
}
