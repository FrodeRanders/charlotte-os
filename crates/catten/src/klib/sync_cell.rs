//! A minimal `Sync` wrapper around `UnsafeCell` for statics whose ownership
//! discipline is architectural (one logical processor per slot, or a
//! boot-time single writer) rather than enforced by the type system.

use core::cell::UnsafeCell;

/// `UnsafeCell` that can be placed in a `static`.
///
/// Every accessor returns a raw pointer; callers must uphold the same
/// aliasing rules as with any `UnsafeCell`, and must ensure no two agents
/// mutate one slot concurrently unless the inner type provides its own
/// synchronization.
#[repr(transparent)]
pub struct SyncUnsafeCell<T>(UnsafeCell<T>);

// SAFETY: the wrapper only hands out raw pointers. Sharing `&SyncUnsafeCell`
// across threads is sound for the same reason `UnsafeCell` access is
// sound when the caller follows the ownership contract documented above.
unsafe impl<T> Sync for SyncUnsafeCell<T> {}

impl<T> SyncUnsafeCell<T> {
    pub const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    pub const fn get(&self) -> *mut T {
        self.0.get()
    }
}
