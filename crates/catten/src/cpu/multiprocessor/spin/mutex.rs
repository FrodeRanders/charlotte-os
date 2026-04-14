use core::ops::{Deref, DerefMut};

use lock_api::RawMutex;

use crate::cpu::isa::lp::ops::{mask_interrupts, unmask_interrupts};

#[derive(Debug)]
pub struct MutexGuard<'a, T>(spin::MutexGuard<'a, T>);

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        unmask_interrupts!()
    }
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = spin::MutexGuard<'a, T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug)]
pub struct Mutex<T>(spin::Mutex<T>);

impl<'a, T> Mutex<T> {
    pub const fn new(data: T) -> Self {
        Mutex(spin::Mutex::new(data))
    }
}

unsafe impl<'a> RawMutex for Mutex<()> {
    type GuardMarker = lock_api::GuardNoSend;

    const INIT: Self = Self::new(());

    fn lock(&'a self) -> spin::MutexGuard<'a, ()> {
        let guard = self.0.lock();
        mask_interrupts!();
        guard
    }

    fn is_locked(&self) -> bool {
        self.0.is_locked()
    }

    fn try_lock(&'a self) -> bool {
        if let Some(guard) = self.0.try_lock() {
            mask_interrupts!();
            true
        } else {
            false
        }
    }

    unsafe fn unlock(&self) {
        unsafe {
            self.0.unlock();
        }
        unmask_interrupts!();
    }
}
