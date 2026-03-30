use alloc::vec::Vec;
use core::sync::atomic::AtomicBool;

use super::{Observable, Observer};

pub struct Any<'a> {
    observers: spin::Mutex<Vec<&'a dyn Observer>>,
}

impl<'a> Observable<'a> for Any<'a> {
    fn register_observer(&'a self, observer: &'a dyn Observer) {
        self.observers.lock().push(observer);
    }
}

impl Observer for Any<'_> {
    fn notify(&self) {
        for observer in self.observers.lock().iter() {
            observer.notify();
        }
    }
}

pub struct All<'a> {
    lock: AtomicBool,
    observers: Vec<&'a dyn Observer>,
}
