use core::sync::atomic::{AtomicI64, Ordering};

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
}

impl RwLockCore {
    pub const fn new() -> Self {
        Self {
            state: AtomicI64::new(0),
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
            let state = self.state.load(Ordering::Acquire);
            if state >= 0 {
                // Try to acquire the lock for reading by incrementing the reader count.
                if self
                    .state
                    .compare_exchange(state, state + 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    break; // Successfully acquired the lock for reading.
                }
            } else {
                // The lock is held by a writer, so we need to wait.
                core::hint::spin_loop();
            }
        }
        INT_STATE.save_int();
    }

    fn try_lock_shared(&self) -> bool {
        let state = self.state.load(Ordering::Acquire);
        if state >= 0 {
            // Try to acquire the lock for reading by incrementing the reader count.
            let ret = self
                .state
                .compare_exchange(state, state + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok();
            if ret {
                INT_STATE.save_int();
            }
            ret
        } else {
            false // The lock is held by a writer, so we cannot acquire it for reading.
        }
    }

    unsafe fn unlock_shared(&self) {
        // Decrement the reader count to release the lock for reading.
        self.state.fetch_sub(1, Ordering::Release);
        INT_STATE.restore_int();
    }

    fn try_lock_exclusive(&self) -> bool {
        let state = self.state.load(Ordering::Acquire);
        if state == 0 {
            // Try to acquire the lock for writing by setting it to -1.
            let ret =
                self.state.compare_exchange(0, -1, Ordering::AcqRel, Ordering::Acquire).is_ok();
            if ret {
                INT_STATE.save_int();
            }
            ret
        } else {
            false // The lock is held by readers or a writer, so we cannot acquire it for writing.
        }
    }

    fn lock_exclusive(&self) {
        loop {
            let state = self.state.load(Ordering::Acquire);
            if state == 0 {
                // Try to acquire the lock for writing by setting it to -1.
                if self.state.compare_exchange(0, -1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    break; // Successfully acquired the lock for writing.
                }
            } else {
                // The lock is held by readers or a writer, so we need to wait.
                core::hint::spin_loop();
            }
        }
        INT_STATE.save_int();
    }

    unsafe fn unlock_exclusive(&self) {
        // Set the state back to 0 to release the lock for writing.
        self.state.store(0, Ordering::Release);
        INT_STATE.restore_int();
    }
}
