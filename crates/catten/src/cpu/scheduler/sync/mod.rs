//! Scheduler-integrated blocking synchronisation primitives.
//!
//! Unlike the raw spin locks in `cpu::multiprocessor::spin`, these wrap
//! the scheduler's blocking mechanism: a contended `lock()` calls
//! `block_thread` on the caller and registers a waker that re-admits the
//! thread when the lock holder calls `unlock()`.  This is cooperative
//! blocking — the caller yields the LP rather than spinning.

pub mod mutex;
pub mod rwlock;
pub(crate) mod tests;

/// Debugger-visible runnable retries after lock-wait admission rejection.
/// Mutex, shared RwLock, exclusive RwLock. These counters do not drive policy.
#[unsafe(no_mangle)]
pub static LOCK_WAIT_ADMISSION_RETRIES: [core::sync::atomic::AtomicU64; 3] =
    [const { core::sync::atomic::AtomicU64::new(0) }; 3];
