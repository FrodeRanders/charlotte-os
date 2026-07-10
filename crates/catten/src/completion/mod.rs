//! # Completion-Capability Subsystem (Option C prototype)
//!
//! This module is the first in-kernel prototype of the async syscall /
//! completion-capability ABI specified in `docs/async-syscall-abi.md`. It builds
//! the kernel side of the boundary directly on facilities that already exist:
//!
//! - [`IdTable`](crate::klib::collections::id_table::IdTable) backs a per-address-space
//!   **capability table** mapping a small integer [`CompletionCap`] to a kernel
//!   object naming an in-flight or completed operation;
//! - the [`Observable`]/[`Observer`] mechanism (the same one `TimerEvent` and
//!   thread-exit use) is how a completion signals the threads awaiting it — so
//!   [`wait`] blocks exactly as [`sleep`](crate::cpu::scheduler::sleep) does,
//!   registering the caller's `Waker` as an observer;
//! - an owned `Vec<u8>` transferred on [`submit`] is retained by the kernel
//!   until a terminal completion hands it back — the buffer-ownership /
//!   deferred-reclaim contract mirrored from sitas's `io_uring` discipline.
//!
//! ## Scope and honesty
//!
//! There is **no syscall entry path yet** (`sync_dispatcher` panics on SVC; no
//! x86_64 `SYSCALL` handler) and no userspace-mappable completion-queue ring, so
//! this prototype exposes the five ABI operations as *kernel-internal* functions
//! and is exercised by boot-time self-tests rather than from EL0. [`complete`] is
//! the kernel-side hook a real worker/driver would call when its work finishes;
//! in the future that call site is the exit-observer of the worker thread (see
//! `scheduler::threads::mod.rs:63-69`). The submission-side capability table and
//! the buffer-ownership contract — the *center of gravity* identified in Phase 2
//! — are real here; the shared-memory CQ/SQ rings and the EL0 syscall glue are
//! the remaining, larger pieces.

use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use concurrent_queue::ConcurrentQueue;
use spin::{LazyLock, Mutex, RwLock};

use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::cpu::scheduler::yield_lp;
use crate::klib::observer::{Observable, Observer};
use crate::memory::AddressSpaceId;

/// A per-address-space handle naming an in-flight or completed async operation.
///
/// This is the kernel realization of the ABI's `Handle` — the value that would
/// cross the syscall boundary. It is an index into the owning address space's
/// capability table.
pub type CompletionCap = usize;

/// The operation an async syscall performs. Only enough variants to exercise the
/// buffer-ownership contract are modelled in this prototype.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    /// No-op completion (no buffer transfer); models a pure signal.
    Nop,
    /// Read into the submitted buffer (buffer returned on completion).
    Read,
    /// Write from the submitted buffer (buffer returned on completion).
    Write,
}

/// The terminal result carried by a completed capability.
#[derive(Debug, PartialEq, Eq)]
pub enum OpResult {
    /// Success with an operation-specific value (for example bytes transferred).
    Ok(i64),
    /// Failure with an error code.
    Err(i32),
    /// The operation reached a terminal state after [`cancel`].
    Cancelled,
}

/// The drained outcome of a completed capability: the terminal result plus any
/// buffer handed back to the owner.
#[derive(Debug)]
pub struct Completed {
    /// The terminal result.
    pub result: OpResult,
    /// The buffer whose ownership returns to the caller, mirroring sitas's
    /// `WriteAtUringCompletion { bytes, buffer }`.
    pub buffer: Option<Vec<u8>>,
}

/// The state of a [`cancel`] request.
#[derive(Debug, PartialEq, Eq)]
pub enum CancelState {
    /// The operation had already completed; nothing to cancel.
    AlreadyComplete,
    /// Cancellation was requested; a terminal completion will still be posted,
    /// and any transferred buffer is retained until then (deferred reclaim).
    CancelRequested,
}

/// Reason a [`submit`] was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum SubmitError {
    /// The capability table is full — first-class, non-fatal backpressure.
    WouldBlock,
    /// No capability table is open for this address space.
    UnknownAddressSpace,
}

/// Reason an operation on an existing capability failed.
#[derive(Debug, PartialEq, Eq)]
pub enum CapError {
    /// No capability table is open for this address space.
    UnknownAddressSpace,
    /// No such capability in the address space's table.
    UnknownCap,
    /// The capability's operation has not reached a terminal completion yet.
    NotComplete,
}

struct CompletionInner {
    buffer: Option<Vec<u8>>,
    result: Option<OpResult>,
    cancelling: bool,
}

/// A kernel object naming one in-flight or completed operation.
///
/// It is [`Observable`]: threads awaiting the operation register their `Waker`
/// here (via [`wait`]), and [`Completion::complete`] notifies them — the exact
/// pattern `TimerEvent` uses for `sleep`.
pub struct Completion {
    inner: Mutex<CompletionInner>,
    observers: ConcurrentQueue<Weak<dyn Observer>>,
}

impl Completion {
    fn new(buffer: Option<Vec<u8>>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(CompletionInner {
                buffer,
                result: None,
                cancelling: false,
            }),
            observers: ConcurrentQueue::unbounded(),
        })
    }

    fn is_complete(&self) -> bool {
        self.inner.lock().result.is_some()
    }

    fn holds_buffer(&self) -> bool {
        self.inner.lock().buffer.is_some()
    }

    /// Kernel-side hook: the operation finished. Records the terminal result
    /// (forced to [`OpResult::Cancelled`] if a cancel was requested) and wakes
    /// every awaiting observer. Idempotent: a second call is a no-op.
    fn complete(&self, result: OpResult) {
        {
            let mut inner = self.inner.lock();
            if inner.result.is_some() {
                return;
            }
            inner.result = Some(if inner.cancelling {
                OpResult::Cancelled
            } else {
                result
            });
        }
        self.signal();
    }

    fn signal(&self) {
        for observer in self.observers.try_iter() {
            if let Some(observer) = observer.upgrade() {
                observer.notify();
            }
        }
    }

    /// Drains the terminal result and returns the buffer to the caller. Returns
    /// `None` while the operation is still in flight.
    fn take(&self) -> Option<Completed> {
        let mut inner = self.inner.lock();
        inner.result.take().map(|result| Completed {
            result,
            buffer: inner.buffer.take(),
        })
    }

    /// Requests cancellation. If already complete, reports it; otherwise marks
    /// the operation cancelling. The buffer is deliberately retained until a
    /// terminal completion is posted (deferred reclaim).
    fn cancel(&self) -> CancelState {
        let mut inner = self.inner.lock();
        if inner.result.is_some() {
            CancelState::AlreadyComplete
        } else {
            inner.cancelling = true;
            CancelState::CancelRequested
        }
    }
}

impl Observable for Completion {
    fn register_observer(&self, observer: Weak<dyn Observer>) {
        // A closed/overflowing observer queue is not fatal: a missed
        // registration only means a caller must fall back to polling.
        let _ = self.observers.push(observer);
    }
}

struct AsCompletions {
    table: crate::klib::collections::id_table::IdTable<Arc<Completion>>,
    capacity: usize,
    live: usize,
}

/// Per-address-space capability tables. In a full design these would live inside
/// each `AddressSpace`; keying them here keeps the prototype self-contained
/// without modifying the address-space type.
static COMPLETIONS: LazyLock<RwLock<BTreeMap<AddressSpaceId, AsCompletions>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));

/// Opens a bounded capability table for an address space. `capacity` bounds the
/// number of concurrently in-flight capabilities (submission backpressure).
pub fn open_address_space(asid: AddressSpaceId, capacity: usize) {
    COMPLETIONS.write().insert(
        asid,
        AsCompletions {
            table: crate::klib::collections::id_table::IdTable::new(),
            capacity,
            live: 0,
        },
    );
}

/// Closes an address space's capability table, dropping any outstanding
/// completions (and, with them, any buffers the kernel still owned — the
/// drain-or-leak teardown case, here a clean reap with the address space).
pub fn close_address_space(asid: AddressSpaceId) {
    COMPLETIONS.write().remove(&asid);
}

fn completion_of(asid: AddressSpaceId, cap: CompletionCap) -> Result<Arc<Completion>, CapError> {
    let registry = COMPLETIONS.read();
    let as_completions = registry.get(&asid).ok_or(CapError::UnknownAddressSpace)?;
    let completion = as_completions
        .table
        .get(cap)
        .map_err(|_| CapError::UnknownCap)?;
    Ok(completion.clone())
}

/// Starts an async operation. Returns immediately with a capability naming it;
/// ownership of `buffer` transfers to the kernel until a terminal completion is
/// posted. Returns [`SubmitError::WouldBlock`] under submission backpressure.
pub fn submit(
    asid: AddressSpaceId,
    _op: OpCode,
    buffer: Option<Vec<u8>>,
) -> Result<CompletionCap, SubmitError> {
    let mut registry = COMPLETIONS.write();
    let as_completions = registry
        .get_mut(&asid)
        .ok_or(SubmitError::UnknownAddressSpace)?;
    if as_completions.live >= as_completions.capacity {
        return Err(SubmitError::WouldBlock);
    }
    let cap = as_completions.table.add_element(Completion::new(buffer));
    as_completions.live += 1;
    Ok(cap)
}

/// Kernel-side completion hook: the worker/driver executing `cap`'s operation
/// finished. Posts the terminal result and wakes any awaiting thread. In a full
/// design this is invoked from the completing work's exit-observer.
pub fn complete(asid: AddressSpaceId, cap: CompletionCap, result: OpResult) -> Result<(), CapError> {
    let completion = completion_of(asid, cap)?;
    completion.complete(result);
    Ok(())
}

/// Non-blocking check: drains and returns the completion if it is terminal,
/// otherwise `Ok(None)`. Handing back the buffer transfers ownership to the
/// caller.
pub fn poll(asid: AddressSpaceId, cap: CompletionCap) -> Result<Option<Completed>, CapError> {
    let completion = completion_of(asid, cap)?;
    Ok(completion.take())
}

/// Blocks the calling thread until `cap` reaches a terminal completion.
///
/// This mirrors [`sleep`](crate::cpu::scheduler::sleep): it registers the
/// caller's `Waker` as an observer of the completion and yields. The re-check
/// after blocking closes the lost-wake race in which the operation completes
/// between the fast-path check and `block_thread`.
pub fn wait(asid: AddressSpaceId, cap: CompletionCap) -> Result<(), CapError> {
    let completion = completion_of(asid, cap)?;
    if completion.is_complete() {
        return Ok(());
    }

    let tid = SYSTEM_SCHEDULER
        .read()
        .get_lp_scheduler()
        .lock()
        .get_tid()
        .ok_or(CapError::UnknownCap)?;

    SYSTEM_SCHEDULER
        .write()
        .block_thread(tid, completion.as_ref() as &dyn Observable)
        .map_err(|_| CapError::UnknownCap)?;

    // Lost-wake guard: if the operation completed after our fast-path check but
    // before (or during) registration, make the thread runnable again.
    if completion.is_complete() {
        let _ = SYSTEM_SCHEDULER.write().submit_ready_thread(tid);
    }

    yield_lp();
    Ok(())
}

/// Requests cancellation of an in-flight operation. See [`CancelState`]. Any
/// transferred buffer is retained until the terminal completion hands it back.
pub fn cancel(asid: AddressSpaceId, cap: CompletionCap) -> Result<CancelState, CapError> {
    let completion = completion_of(asid, cap)?;
    Ok(completion.cancel())
}

/// Releases a completed capability's table slot. Fails with
/// [`CapError::NotComplete`] if the operation is still in flight.
pub fn close(asid: AddressSpaceId, cap: CompletionCap) -> Result<(), CapError> {
    let completion = completion_of(asid, cap)?;
    if !completion.is_complete() {
        return Err(CapError::NotComplete);
    }
    let mut registry = COMPLETIONS.write();
    let as_completions = registry.get_mut(&asid).ok_or(CapError::UnknownAddressSpace)?;
    as_completions
        .table
        .remove_element(cap)
        .map_err(|_| CapError::UnknownCap)?;
    as_completions.live = as_completions.live.saturating_sub(1);
    Ok(())
}

/// Registers an observer to be notified when `cap` completes — the same
/// mechanism [`wait`] uses internally, exposed so userspace (or a self-test) can
/// monitor a capability in real time.
pub fn observe(
    asid: AddressSpaceId,
    cap: CompletionCap,
    observer: Weak<dyn Observer>,
) -> Result<(), CapError> {
    let completion = completion_of(asid, cap)?;
    completion.register_observer(observer);
    Ok(())
}

/// Test/inspection helper: whether the kernel still owns a buffer for `cap`
/// (i.e. it has not yet been handed back). Demonstrates deferred reclaim.
pub fn holds_buffer(asid: AddressSpaceId, cap: CompletionCap) -> Result<bool, CapError> {
    Ok(completion_of(asid, cap)?.holds_buffer())
}
