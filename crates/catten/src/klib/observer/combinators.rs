use alloc::sync::Weak;
use alloc::vec::Vec;
use core::sync::atomic::AtomicBool;

use super::{Observable, Observer};

pub struct Any {
    observers: spin::Mutex<Vec<Weak<dyn Observer>>>,
}

impl Observable for Any {
    fn register_observer(&mut self, observer: Weak<dyn Observer>) {
        self.observers.lock().push(observer);
    }
}

impl Observer for Any {
    fn notify(&self) {
        for observer in self.observers.lock().iter() {
            if let Some(observer) = observer.upgrade() {
                observer.notify();
            }
        }
    }
}

pub struct All<'a> {
    lock: AtomicBool,
    observers: Vec<&'a dyn Observer>,
}
