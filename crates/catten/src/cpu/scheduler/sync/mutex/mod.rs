use alloc::sync::Weak;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use concurrent_queue::ConcurrentQueue;

use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::klib::observer::{Observable, Observer};

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<'a, T> MutexGuard<'a, T> {
    fn new(mutex: &'a Mutex<T>) -> Self {
        Self {
            mutex,
        }
    }

    pub fn get(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }

    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.state.store(false, Ordering::Release);
    }
}

pub struct Mutex<T> {
    state: AtomicBool,
    data: UnsafeCell<T>,
    waiters: ConcurrentQueue<Weak<dyn Observer>>,
}

impl<T> Observable for Mutex<T> {
    fn register_observer(&mut self, observer: Weak<dyn crate::klib::observer::Observer>) {
        self.waiters.push(observer).expect("Failed to register observer with busy Mutex");
    }
}

impl<T> Mutex<T> {
    pub const fn new(data: T) -> Self {
        Self {
            state: AtomicBool::new(false),
            data: UnsafeCell::new(data),
            waiters: ConcurrentQueue::unbounded(),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        while self
            .state
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            let tid = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid();
        }
        MutexGuard::new(self)
    }
}
