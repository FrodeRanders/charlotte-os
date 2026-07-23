//! Scheduler-integrated blocking synchronisation primitives.
//!
//! Unlike the raw spin locks in `cpu::multiprocessor::spin`, these wrap
//! the scheduler's blocking mechanism: a contended `lock()` calls
//! `block_thread` on the caller and registers a waker that re-admits the
//! thread when the lock holder calls `unlock()`.  This is cooperative
//! blocking — the caller yields the LP rather than spinning.

pub mod mutex;
pub mod rwlock;
