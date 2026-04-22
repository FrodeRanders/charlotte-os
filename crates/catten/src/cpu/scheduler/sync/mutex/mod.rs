use alloc::sync::Weak;
use core::sync::atomic::{AtomicBool, Ordering};

use concurrent_queue::ConcurrentQueue;
use lock_api::{GuardNoSend, RawMutex};

use crate::cpu::scheduler::system_scheduler::{SYSTEM_SCHEDULER, get_thread_id};
use crate::klib::observer::{Observable, Observer};

#[derive(Debug)]
struct MutexCore {
    raw_lock: AtomicBool,
    waitlist: ConcurrentQueue<Weak<dyn Observer>>,
}

impl Default for MutexCore {
    fn default() -> Self {
        Self::new()
    }
}

impl MutexCore {
    pub fn new() -> Self {
        MutexCore {
            raw_lock: AtomicBool::new(false),
            waitlist: ConcurrentQueue::unbounded(),
        }
    }
}

impl Observable for MutexCore {
    fn register_observer(&self, observer: Weak<dyn Observer>) {
        self.waitlist.push(observer);
    }
}

unsafe impl RawMutex for MutexCore {
    type GuardMarker = GuardNoSend;

    const INIT: Self = MutexCore {
        raw_lock: AtomicBool::new(false),
        waitlist: ConcurrentQueue::unbounded(),
    };

    fn lock(&self) {
        loop {
            if let Some(tid) = get_thread_id() {
                SYSTEM_SCHEDULER.write().block_thread(tid, self);
            } else {
                panic!("Attempted to acquire a blocking mutex from outside thread context.");
            }
            if self
                .raw_lock
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.waitlist.pop();
                break;
            }
        }
    }

    fn is_locked(&self) -> bool {
        self.raw_lock.load(Ordering::Acquire)
    }

    fn try_lock(&self) -> bool {
        self.waitlist.is_empty()
            && self
                .raw_lock
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }

    unsafe fn unlock(&self) {
        self.raw_lock.store(false, Ordering::Release);
    }
}
