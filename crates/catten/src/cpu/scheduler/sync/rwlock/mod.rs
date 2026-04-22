use alloc::sync::Weak;
use core::sync::atomic::{AtomicI64, Ordering};

use concurrent_queue::ConcurrentQueue;
use lock_api::RawRwLock;

use crate::klib::observer::{Observable, Observer};

pub struct RwLockCore {
    raw_lock: AtomicI64,
    waitlist_shared: ConcurrentQueue<Weak<dyn Observer>>,
    waitlist_exclusive: ConcurrentQueue<Weak<dyn Observer>>,
}

impl Default for RwLockCore {
    fn default() -> Self {
        RwLockCore {
            raw_lock: AtomicI64::new(0),
            waitlist_shared: ConcurrentQueue::unbounded(),
            waitlist_exclusive: ConcurrentQueue::unbounded(),
        }
    }
}

impl RwLockCore {
    pub fn new() -> Self {
        Self::default()
    }
}

unsafe impl RawRwLock for RwLockCore {
    fn is_locked(&self) -> bool {
        self.raw_lock.load(Ordering::Acquire) > 0
    }

    fn is_locked_exclusive(&self) -> bool {
        self.raw_lock.load(Ordering::Acquire) < 0
    }

    fn lock_exclusive(&self) {
        if self.raw_lock.compare_exchange(0, -1, Ordering::AcqRel, Ordering::Acquire).is_err() {}
    }
}
