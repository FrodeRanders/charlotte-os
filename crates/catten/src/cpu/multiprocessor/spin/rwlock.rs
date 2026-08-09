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
    /// Number of writers waiting for the lock. Gives writers preference:
    /// once a writer is queued, new readers must wait for it, so a continuous
    /// stream of readers cannot starve the writer indefinitely.
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
    type GuardMarker = lock_api::GuardSend;

    const INIT: Self = Self::new();

    fn lock_shared(&self) {
        INT_STATE.save_int();
        loop {
            // Writer preference: once a writer is waiting, new readers must
            // wait for it. Otherwise a dense reader stream (e.g. the IPC
            // registry probes around a reply) can starve the writer forever.
            if self.waiting_writers.load(Ordering::Acquire) != 0 {
                core::hint::spin_loop();
                continue;
            }
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
        INT_STATE.save_int();
        // Announce the waiting writer so new readers defer to it.
        self.waiting_writers.fetch_add(1, Ordering::AcqRel);
        loop {
            let state = self.state.load(Ordering::Acquire);
            if state == 0 {
                // Try to acquire the lock for writing by setting it to -1.
                if self.state.compare_exchange(0, -1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    self.waiting_writers.fetch_sub(1, Ordering::AcqRel);
                    break; // Successfully acquired the lock for writing.
                }
            } else {
                // The lock is held by readers or a writer, so we need to wait.
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
