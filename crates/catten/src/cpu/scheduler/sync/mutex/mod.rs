use alloc::sync::Weak;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};

use concurrent_queue::ConcurrentQueue;

use crate::cpu::scheduler::system_scheduler::{SYSTEM_SCHEDULER, get_thread_id};
use crate::cpu::scheduler::threads::ThreadId;
use crate::klib::observer::{Observable, Observer};

pub struct MutexGuard<'a, T> {
    mutex: &'a mut Mutex<T>,
}

impl<'a, T> MutexGuard<'a, T> {
    fn new(mutex: &'a mut Mutex<T>) -> Self {
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
        unsafe {
            self.mutex.unlock();
        }
    }
}

pub struct Mutex<T> {
    data: UnsafeCell<T>,
    waiters: ConcurrentQueue<(ThreadId, Weak<dyn Observer>)>,
    holder: Option<ThreadId>,
}

impl<T> Observable for Mutex<T> {
    fn register_observer(&mut self, observer: Weak<dyn crate::klib::observer::Observer>) {
        self.waiters
            .push((
                get_thread_id().expect("Attempted to unwrap thread ID from non-thread context"),
                observer,
            ))
            .expect("Failed to register observer with busy Mutex");
    }
}

impl<T> Mutex<T> {
    pub const fn new(data: T) -> Self {
        Self {
            data: UnsafeCell::new(data),
            waiters: ConcurrentQueue::unbounded(),
            holder: None,
        }
    }

    pub fn lock<'m>(&'m mut self) -> MutexGuard<'m, T> {
        let tid = SYSTEM_SCHEDULER
            .read()
            .get_lp_scheduler()
            .lock()
            .get_tid()
            .expect("Attempted unwrap thread ID from non-thread context");
        while !self.waiters.is_empty() || self.holder == Some(tid) {
            SYSTEM_SCHEDULER
                .write()
                .block_thread(tid, self as &mut dyn Observable)
                .expect("Failed to block thread on attempt to lock Mutex");
        }
        MutexGuard::new(self)
    }

    unsafe fn unlock(&mut self) {
        while let Ok(waiter) = self.waiters.pop() {
            if let Some(observer) = waiter.1.upgrade() {
                self.holder = Some(waiter.0);
                observer.notify();
                return;
            }
        }
        self.holder = None;
    }
}
