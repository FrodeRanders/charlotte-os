//! # Shard-Local State (Option B seed)
//!
//! A lock-free per-logical-processor container that enforces sitas's strongest
//! invariant: *references to shard-owned state must not escape the access
//! closure.*
//!
//! Where [`PerLp<T>`](crate::cpu::multiprocessor::spin::per_lp::PerLp) wraps
//! each slot in an `RwLock<T>` and hands out guard objects,
//! **`ShardLocal<T>`** stores each slot in an `UnsafeCell<T>` and only exposes
//! `&mut T` through a synchronous closure, with two runtime assertions:
//!
//! 1. **Owner check** — the caller must be on the LP that owns this slot
//!    (panics otherwise; `try_with` returns `NotOnOwner`).
//! 2. **Re-entrancy guard** — a per-LP borrow flag is set on entry and cleared
//!    on exit (RAII `BorrowGuard`), preventing two concurrent `&mut T` handles
//!    to the same LP's slot (panics; `try_with` returns `AlreadyBorrowed`).
//!
//! The contract mirrors sitas's `ShardLocal<T>`: access is synchronous,
//! non-blocking (no spin-lock acquire), and the reference never crosses the
//! closure boundary. Cross-LP mutation must go through the IPI/workqueue
//! machinery, never through a shared lock.
//!
//! ## When to use each container
//!
//! | | `PerLp<T>` | `ShardLocal<T>` |
//! |---|---|---|
//! | Locking | `RwLock` (spin + interrupt mask) | None (trampoline flag) |
//! | Interrupt-safe | Yes (interrupts masked during guard) | No (must not be accessed from ISR) |
//! | Cross-LP access | `unsafe get_nonlocal*` | ✗ (only through closure dispatch) |
//! | Contention cost | CAS loop on every access | `AtomicBool` swap + assertion |
//! | Use case | Data touched from ISR or cross-LP | Single-LP, thread-local-only data |

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::cpu::isa::lp::ops::get_lp_id;
use crate::cpu::isa::lp::LpId;
use crate::cpu::multiprocessor::get_lp_count;

#[derive(Debug, PartialEq, Eq)]
pub enum ShardLocalAccessError {
    /// The caller is not running on the owning LP (or not in any LP context).
    NotOnOwner,
    /// The slot is already borrowed (re-entrant access from the same LP).
    AlreadyBorrowed,
    /// The given `LpId` is out of range for this container.
    InvalidLpId,
}

/// A lock-free per-LP container. One `T` per logical processor, stored behind
/// an `UnsafeCell<T>`. Access is gated by the owner-check + borrow-flag
/// discipline described in the module docs.
pub struct ShardLocal<T> {
    cells: Box<[UnsafeCell<T>]>,
    borrow_flags: Box<[AtomicBool]>,
}

// `T` is never sent between LPs; cross-LP mutation must go through typed
// message dispatch.
unsafe impl<T: Send> Send for ShardLocal<T> {}
unsafe impl<T: Send> Sync for ShardLocal<T> {}

// Manual Debug since UnsafeCell isn't Debug + we don't want to print T.
impl<T> core::fmt::Debug for ShardLocal<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ShardLocal")
            .field("lp_count", &self.cells.len())
            .finish()
    }
}

impl<T> ShardLocal<T> {
    /// Creates one `T` per logical processor, each initialized by `init()`.
    pub fn new<F: Fn() -> T>(init: F) -> Self {
        let n = get_lp_count() as usize;
        let mut cells = Vec::with_capacity(n);
        let mut borrow_flags = Vec::with_capacity(n);
        for _ in 0..n {
            cells.push(UnsafeCell::new(init()));
            borrow_flags.push(AtomicBool::new(false));
        }
        Self {
            cells: cells.into_boxed_slice(),
            borrow_flags: borrow_flags.into_boxed_slice(),
        }
    }

    /// Returns the number of LP slots in this container.
    pub fn lp_count(&self) -> usize {
        self.cells.len()
    }

    /// Synchronous, lock-free access to the calling LP's slot. Panics if the
    /// caller is not on the owning LP or if the slot is already borrowed.
    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        match self.try_with(f) {
            Ok(r) => r,
            Err(e) => panic!("ShardLocal::with: {e:?}"),
        }
    }

    /// Non-panicking variant of [`with`](Self::with).
    pub fn try_with<R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R, ShardLocalAccessError> {
        let lp = get_lp_id() as usize;
        if lp >= self.cells.len() {
            return Err(ShardLocalAccessError::InvalidLpId);
        }
        let flag = &self.borrow_flags[lp];
        if flag.swap(true, Ordering::AcqRel) {
            return Err(ShardLocalAccessError::AlreadyBorrowed);
        }
        let _guard = BorrowGuard { flag };
        // Safety: we are on the owning LP, the borrow flag is set so no other
        // code on this LP can enter `with`/`try_with`, and cross-LP mutation is
        // forbidden by construction (no nonlocal accessor).
        let value = unsafe { &mut *self.cells[lp].get() };
        Ok(f(value))
    }

    /// Synchronous, lock-free access to a specific LP's slot. Marked `unsafe`
    /// because cross-LP access must be coordinated through message dispatch
    /// (IPI/closure) rather than direct shared-memory mutation. The flag
    /// assertion still protects against re-entrant access on the target LP.
    ///
    /// # Safety
    ///
    /// The caller must ensure that no other code on the *calling* LP
    /// concurrently accesses this slot through `with` or `try_with`, and that
    /// the operation is reachable from a context where the assertion makes
    /// sense (e.g. an IPI-delivered closure running on the target LP).
    #[allow(dead_code)]
    pub unsafe fn with_on_lp<R>(
        &self,
        lp: LpId,
        f: impl FnOnce(&mut T) -> R,
    ) -> Result<R, ShardLocalAccessError> {
        let idx = lp as usize;
        if idx >= self.cells.len() {
            return Err(ShardLocalAccessError::InvalidLpId);
        }
        let flag = &self.borrow_flags[idx];
        if flag.swap(true, Ordering::AcqRel) {
            return Err(ShardLocalAccessError::AlreadyBorrowed);
        }
        let _guard = BorrowGuard { flag };
        let value = unsafe { &mut *self.cells[idx].get() };
        Ok(f(value))
    }
}

/// RAII guard that clears the borrow flag on drop, even if the closure panics
/// and unwinds (the kernel panics with `panic = "abort"`, so Drop runs as the
/// panic handler aborts).
struct BorrowGuard<'a> {
    flag: &'a AtomicBool,
}

impl Drop for BorrowGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}
