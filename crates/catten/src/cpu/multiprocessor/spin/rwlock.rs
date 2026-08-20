use core::sync::atomic::{
    AtomicI64,
    AtomicUsize,
    Ordering,
};

use lock_api::RawRwLock;

use crate::cpu::multiprocessor::interrupt_tracking::INT_STATE;

pub type RwLock<T> = lock_api::RwLock<RwLockCore, T>;
pub type RwLockReadGuard<'a, T> = lock_api::RwLockReadGuard<'a, RwLockCore, T>;
pub type RwLockWriteGuard<'a, T> = lock_api::RwLockWriteGuard<'a, RwLockCore, T>;

/// A raw spin-based read-write lock core structure for use with `lock_api`.
/// Used to implement RwLock for this kernel.
pub struct RwLockCore {
    /// The state of the lock:
    /// - `0` means the lock is free.
    /// - A positive value `n` means there are `n` readers holding the lock.
    /// - `-1` means the lock is held by a writer.
    state: AtomicI64,
    /// Number of writers currently making an IRQ-masked acquisition attempt.
    /// Readers defer during that attempt without leaving IRQs masked while a
    /// writer waits for another LP.
    waiting_writers: AtomicUsize,
}

impl RwLockCore {
    pub const fn new() -> Self {
        Self {
            state: AtomicI64::new(0),
            waiting_writers: AtomicUsize::new(0),
        }
    }
}

impl Default for RwLockCore {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl RawRwLock for RwLockCore {
    type GuardMarker = lock_api::GuardNoSend;

    const INIT: Self = Self::new();

    fn lock_shared(&self) {
        loop {
            INT_STATE.save_int();
            // Briefly defer to a writer making its atomic acquisition attempt.
            if self.waiting_writers.load(Ordering::Acquire) == 0 {
                let state = self.state.load(Ordering::Acquire);
                // Try to acquire the lock for reading by incrementing the reader count.
                if state >= 0
                    && self
                        .state
                        .compare_exchange(state, state + 1, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                {
                    return;
                }
            }
            INT_STATE.restore_int();
            while self.waiting_writers.load(Ordering::Acquire) != 0
                || self.state.load(Ordering::Acquire) < 0
            {
                core::hint::spin_loop();
            }
        }
    }

    fn try_lock_shared(&self) -> bool {
        INT_STATE.save_int();
        // Do not jump ahead of a queued writer.
        if self.waiting_writers.load(Ordering::Acquire) != 0 {
            INT_STATE.restore_int();
            return false;
        }
        let state = self.state.load(Ordering::Acquire);
        if state >= 0 {
            // Try to acquire the lock for reading by incrementing the reader count.
            if self
                .state
                .compare_exchange(state, state + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                true
            } else {
                INT_STATE.restore_int();
                false
            }
        } else {
            INT_STATE.restore_int();
            false // The lock is held by a writer, so we cannot acquire it for reading.
        }
    }

    unsafe fn unlock_shared(&self) {
        // Decrement the reader count to release the lock for reading.
        self.state.fetch_sub(1, Ordering::Release);
        INT_STATE.restore_int();
    }

    fn try_lock_exclusive(&self) -> bool {
        INT_STATE.save_int();
        let state = self.state.load(Ordering::Acquire);
        if state == 0 {
            // Try to acquire the lock for writing by setting it to -1.
            if self.state.compare_exchange(0, -1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                true
            } else {
                INT_STATE.restore_int();
                false
            }
        } else {
            INT_STATE.restore_int();
            false // The lock is held by readers or a writer, so we cannot acquire it for writing.
        }
    }

    fn lock_exclusive(&self) {
        loop {
            INT_STATE.save_int();
            // Keep writer preference inside the IRQ-masked attempt. Leaving a
            // writer announced while waiting with interrupts enabled could
            // deadlock an interrupt handler that needs a read guard here.
            self.waiting_writers.fetch_add(1, Ordering::AcqRel);
            if self.state.compare_exchange(0, -1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                self.waiting_writers.fetch_sub(1, Ordering::AcqRel);
                return;
            }
            self.waiting_writers.fetch_sub(1, Ordering::AcqRel);
            INT_STATE.restore_int();
            while self.state.load(Ordering::Acquire) != 0 {
                core::hint::spin_loop();
            }
        }
    }

    unsafe fn unlock_exclusive(&self) {
        // Any still-waiting writers keep readers out through waiting_writers.
        self.state.store(0, Ordering::Release);
        INT_STATE.restore_int();
    }
}
