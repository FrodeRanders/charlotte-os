use alloc::sync::Weak;
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use concurrent_queue::ConcurrentQueue;
use lock_api::{
    GuardNoSend,
    RawMutex,
};

use crate::{
    cpu::scheduler::system_scheduler::{
        SYSTEM_SCHEDULER,
        get_thread_id,
    },
    klib::observer::{
        Observable,
        Observer,
    },
};

pub type Mutex<T> = lock_api::Mutex<MutexCore, T>;

#[derive(Debug)]
pub struct MutexCore {
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
        self.waitlist.push(observer).expect("Failed to register observer");
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
            if self
                .raw_lock
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return; // acquired
            }
            let Some(tid) = get_thread_id() else {
                panic!("Attempted to acquire a blocking mutex from outside thread context.");
            };
            let generation = match SYSTEM_SCHEDULER.read().block_thread_with_constraint_generation(
                tid,
                self,
                crate::cpu::scheduler::threads::MigrationConstraint::GeneralWait,
            ) {
                Ok(generation) => generation,
                // Already blocked: a notify raced with the CAS. Yield and retry.
                Err(_) => {
                    crate::cpu::scheduler::yield_lp();
                    continue;
                }
            };
            // Lost-wake guard: unlock may have run between the failed CAS and
            // observer registration, in which case no future unlock is coming.
            if !self.raw_lock.load(Ordering::Acquire) {
                let _ = SYSTEM_SCHEDULER.read().submit_woken_thread(tid, generation);
            }
            crate::cpu::scheduler::yield_lp();
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
        // Wake the next waiter *after* releasing the lock
        while let Ok(observer) = self.waitlist.pop() {
            if let Some(observer) = observer.upgrade() {
                observer.notify();
                break;
            }
        }
    }
}
