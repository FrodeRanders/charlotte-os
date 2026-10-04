use core::sync::atomic::{
    AtomicI64,
    Ordering,
};

use lock_api::RawRwLock;

use crate::{
    cpu::scheduler::system_scheduler::{
        SYSTEM_SCHEDULER,
        get_thread_id,
    },
    klib::observer::waiter_source::WaiterSource,
};

pub type RwLock<T> = lock_api::RwLock<RwLockCore, T>;

#[derive(Default)]
pub struct RwLockCore {
    raw_lock: AtomicI64,
    waitlist_shared: WaiterSource,
    waitlist_exclusive: WaiterSource,
}

impl RwLockCore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Kernel white-box diagnostics use the same sources as normal acquisition.
    pub(super) fn waiter_source(&self, shared: bool) -> &WaiterSource {
        if shared {
            &self.waitlist_shared
        } else {
            &self.waitlist_exclusive
        }
    }
}

impl RwLockCore {
    fn wake_waiters(&self) {
        // Detach both classes before any callback. No single candidate can
        // claim a guaranteed handoff: it may already have been cancelled.
        let writers = self.waitlist_exclusive.drain();
        let readers = self.waitlist_shared.drain();
        writers.notify();
        readers.notify();
    }
}

unsafe impl RawRwLock for RwLockCore {
    type GuardMarker = lock_api::GuardNoSend;

    const INIT: Self = Self {
        raw_lock: AtomicI64::new(0),
        waitlist_shared: WaiterSource::new(),
        waitlist_exclusive: WaiterSource::new(),
    };

    fn lock_exclusive(&self) {
        loop {
            if self.raw_lock.compare_exchange(0, -1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return;
            }
            let Some(tid) = get_thread_id() else {
                panic!("Attempted to lock a blocking lock from outside thread context!");
            };
            let setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
            let generation = match SYSTEM_SCHEDULER.read().block_thread_with_constraint_generation(
                tid,
                &self.waitlist_exclusive,
                crate::cpu::scheduler::threads::MigrationConstraint::GeneralWait,
            ) {
                Ok(generation) => Some(generation),
                Err(error) => {
                    if matches!(
                        error,
                        crate::cpu::scheduler::system_scheduler::Error::WaitRegistrationFailed
                    ) {
                        super::LOCK_WAIT_ADMISSION_RETRIES[2].fetch_add(1, Ordering::Relaxed);
                    }
                    None
                }
            };
            // Lost-wake guard: unlock may have run between the failed CAS and
            // observer registration.
            if let Some(generation) = generation
                && self.raw_lock.load(Ordering::Acquire) == 0
            {
                let _ = SYSTEM_SCHEDULER.read().submit_woken_thread(tid, generation);
            }
            drop(setup);
            crate::cpu::scheduler::yield_lp();
        }
    }

    fn try_lock_exclusive(&self) -> bool {
        !self.raw_lock.compare_exchange(0, -1, Ordering::AcqRel, Ordering::Acquire).is_err()
    }

    unsafe fn unlock_exclusive(&self) {
        if self.raw_lock.compare_exchange(-1, 0, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            self.wake_waiters();
        } else {
            panic!("Attempted to unlock an exclusive lock that was not held!");
        }
    }

    fn lock_shared(&self) {
        loop {
            if self
                .raw_lock
                .try_update(Ordering::AcqRel, Ordering::Acquire, |x| {
                    if x >= 0 {
                        Some(x + 1)
                    } else {
                        None
                    }
                })
                .is_ok()
            {
                return;
            }
            let Some(tid) = get_thread_id() else {
                panic!("Attempted to lock a blocking lock from outside thread context!");
            };
            // As for the writer, park/recheck must not be separated by a
            // quantum: unlock-before-registration can leave no later wake.
            let setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
            let generation = match SYSTEM_SCHEDULER.read().block_thread_with_constraint_generation(
                tid,
                &self.waitlist_shared,
                crate::cpu::scheduler::threads::MigrationConstraint::GeneralWait,
            ) {
                Ok(generation) => Some(generation),
                Err(error) => {
                    if matches!(
                        error,
                        crate::cpu::scheduler::system_scheduler::Error::WaitRegistrationFailed
                    ) {
                        super::LOCK_WAIT_ADMISSION_RETRIES[1].fetch_add(1, Ordering::Relaxed);
                    }
                    None
                }
            };
            if let Some(generation) = generation
                && self.raw_lock.load(Ordering::Acquire) >= 0
            {
                let _ = SYSTEM_SCHEDULER.read().submit_woken_thread(tid, generation);
            }
            drop(setup);
            crate::cpu::scheduler::yield_lp();
        }
    }

    fn try_lock_shared(&self) -> bool {
        self.raw_lock
            .try_update(Ordering::AcqRel, Ordering::Acquire, |x| {
                if x >= 0 {
                    Some(x + 1)
                } else {
                    None
                }
            })
            .is_ok()
    }

    unsafe fn unlock_shared(&self) {
        match self.raw_lock.try_update(Ordering::AcqRel, Ordering::Acquire, |x| {
            if x > 0 {
                Some(x - 1)
            } else {
                None
            }
        }) {
            Ok(1) => self.wake_waiters(),
            Ok(_) => {} // More readers still own it; a writer cannot acquire yet.
            Err(_) => panic!("Attempted to unlock a shared lock that was not held!"),
        }
    }
}
