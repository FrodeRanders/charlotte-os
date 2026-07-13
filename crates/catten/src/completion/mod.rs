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
//! The [`cq`] submodule provides a shared-memory completion-queue ring (io_uring-
//! style) for zero-syscall completion delivery to userspace.
//!
//! ## Scope and honesty
//!
//! The AArch64 syscall entry path is wired (`sync_dispatcher` decodes SVC,
//! dispatches to the syscall table, and a real-EL0 test thread exercises the
//! round-trip). The CQ ring in [`cq`] is the next layer: mapping it into a user
//! address space enables zero-syscall completion draining from userspace.
//! The submission-side capability table and buffer-ownership contract are real;
//! [`complete`] is the kernel-side hook a worker's exit-observer would call.

pub mod cq;

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Set when take() consumes the result. A cap is reclaimable iff
    /// `result.is_some()` (terminal result posted) OR `drained` (result
    /// already consumed by a prior take()).
    drained: bool,
    /// Keeps the exit-observer (if any) alive for as long as the capability
    /// exists. `observe_thread_exit` stores only a `Weak`, so the strong `Arc`
    /// must live somewhere; it lives here, so the observer can still fire when
    /// the worker thread exits.
    exit_observer: Option<Arc<CompletionExitObserver>>,
}

/// An [`Observer`] that completes a capability when the worker thread it is
/// registered against exits. This is the ABI's intended completion mechanism
/// (see `scheduler/threads/mod.rs:63-69`): a completion capability is registered
/// as an exit-observer of the thread performing the work, so the thread exiting
/// *is* the completion event.
struct CompletionExitObserver {
    asid: AddressSpaceId,
    cap: CompletionCap,
    /// The result to post when the thread exits.
    result: OpResult,
}

impl Observer for CompletionExitObserver {
    fn notify(self: Arc<Self>) {
        // The worker thread has exited: post the terminal result. This runs
        // from the reaper (`reap_dead_threads` in `cond_yield_lp`), which holds
        // no scheduler locks, so waking a waiter via `complete` is safe.
        let _ = complete(self.asid, self.cap, self.result.clone());
    }
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
                drained: false,
                exit_observer: None,
            }),
            observers: ConcurrentQueue::unbounded(),
        })
    }

    fn set_exit_observer(&self, observer: Arc<CompletionExitObserver>) {
        self.inner.lock().exit_observer = Some(observer);
    }

    fn is_reclaimable(&self) -> bool {
        let inner = self.inner.lock();
        inner.result.is_some() || inner.drained
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
    /// `None` while the operation is still in flight. Sets the `drained` flag
    /// so the cap remains reclaimable after the result is consumed.
    fn take(&self) -> Option<Completed> {
        let mut inner = self.inner.lock();
        let result = inner.result.take();
        if result.is_some() {
            inner.drained = true;
        }
        result.map(|r| Completed {
            result: r,
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
    /// Optional per-AS completion-queue ring (zero-syscall drain path).
    /// The allocation backing the ring is kept alive by `_cq_buf`.
    cq_ring: Option<*mut crate::completion::cq::CompletionQueueRing>,
    #[allow(dead_code)]
    _cq_buf: Option<alloc::boxed::Box<alloc::vec::Vec<u8>>>,
}

// AsCompletions stores raw pointers to CQ rings allocated from the kernel heap;
// all access goes through COMPLETIONS' RwLock, so concurrent access is
// serialised. Vec<u8> is not Sync, but we store it behind a Box which is
// accessed only from within the RwLock.
unsafe impl Send for AsCompletions {}
unsafe impl Sync for AsCompletions {}

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
            cq_ring: None,
            _cq_buf: None,
        },
    );
}

/// Like [`open_address_space`] but also allocates and attaches a per-AS
/// completion-queue ring. The ring is a single 4 KiB page with `cq_entries`
/// entry slots, accessible from the kernel via the raw pointer stored in the
/// registry.
pub fn open_address_space_with_cq(
    asid: AddressSpaceId,
    cap_table_capacity: usize,
    cq_entries: u32,
) {
    let (buf, ring_ptr) = crate::completion::cq::CompletionQueueRing::new_page(cq_entries);
    COMPLETIONS.write().insert(
        asid,
        AsCompletions {
            table: crate::klib::collections::id_table::IdTable::new(),
            capacity: cap_table_capacity,
            live: 0,
            cq_ring: Some(ring_ptr),
            _cq_buf: Some(alloc::boxed::Box::new(buf)),
        },
    );
}

/// Returns a raw pointer to the CQ ring for `asid`, or `None`.
/// Like [`open_address_space_with_cq`] but initialises the ring on a
/// pre-allocated physical frame (for mappings where the same frame must also
/// appear in a user page table).
pub fn open_address_space_with_cq_phys(
    asid: AddressSpaceId,
    cap_table_capacity: usize,
    ring_frame: crate::memory::physical::PAddr,
    cq_entries: u32,
) {
    let ring_ptr = unsafe {
        crate::completion::cq::CompletionQueueRing::init_at_phys(ring_frame, cq_entries)
    };
    COMPLETIONS.write().insert(
        asid,
        AsCompletions {
            table: crate::klib::collections::id_table::IdTable::new(),
            capacity: cap_table_capacity,
            live: 0,
            cq_ring: Some(ring_ptr),
            _cq_buf: None,
        },
    );
}

/// Returns a raw pointer to the CQ ring for `asid`, or `None`.
pub fn cq_ring_of(asid: AddressSpaceId) -> Option<*mut crate::completion::cq::CompletionQueueRing> {
    let registry = COMPLETIONS.read();
    registry.get(&asid).and_then(|c| c.cq_ring)
}

/// Closes an address space's capability table and frees its CQ ring (if any).
pub fn close_address_space(asid: AddressSpaceId) {
    COMPLETIONS.write().remove(&asid);
}

pub fn completion_of(asid: AddressSpaceId, cap: CompletionCap) -> Result<Arc<Completion>, CapError> {
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

/// Submits an operation that is performed by a freshly spawned kernel worker
/// thread, and returns a capability that completes **when the worker thread
/// exits**.
///
/// This is the ABI's intended asynchronous-completion mechanism (see
/// `scheduler/threads/mod.rs:63-69`): the returned capability is registered as
/// an exit-observer of the worker thread, so the worker simply performs its work
/// and returns — the thread exiting *is* the completion event, which fires the
/// capability and wakes any waiter. The worker does not touch the capability.
///
/// `result` is the terminal result posted when the worker exits.
pub fn submit_worker(
    asid: AddressSpaceId,
    worker_entry: extern "C" fn(),
    result: OpResult,
) -> Result<CompletionCap, SubmitError> {
    let cap = submit(asid, OpCode::Nop, None)?;
    // Spawn the worker that performs the operation.
    let tid = crate::cpu::scheduler::spawn_thread(crate::memory::KERNEL_ASID, worker_entry);
    // Register an exit-observer that completes the capability when the worker
    // exits, and keep the observer alive by storing it in the completion.
    let observer = Arc::new(CompletionExitObserver { asid, cap, result });
    let _ = crate::cpu::scheduler::observe_thread_exit(
        tid,
        Arc::downgrade(&observer) as Weak<dyn Observer>,
    );
    if let Ok(completion) = completion_of(asid, cap) {
        completion.set_exit_observer(observer);
    }
    Ok(cap)
}

/// Kernel-side completion hook: the worker/driver executing `cap`'s operation
/// finished. Posts the terminal result, wakes any awaiting thread, and writes an
/// entry to the AS's CQ ring if one is attached.
pub fn complete(asid: AddressSpaceId, cap: CompletionCap, result: OpResult) -> Result<(), CapError> {
    let completion = completion_of(asid, cap)?;

    // Publish the completion entry to the shared CQ ring *before* waking any
    // waiter. A userspace consumer that blocks in `wait` and then drains the
    // ring the moment it is woken must observe the entry, so the ring write has
    // to happen-before the wake, not after it.
    if let Some(ring_ptr) = cq_ring_of(asid) {
        unsafe { &mut *ring_ptr }.write(cap, result.clone());
    }

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

/// Releases a completed or already-drained capability slot. Fails with
/// [`CapError::NotComplete`] if the operation is still in flight (neither
/// completed nor drained).
pub fn close(asid: AddressSpaceId, cap: CompletionCap) -> Result<(), CapError> {
    let completion = completion_of(asid, cap)?;
    if !completion.is_reclaimable() {
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

/// Polls the CQ ring for `asid` and returns the number of pending entries.
/// Returns 0 if no CQ ring is attached.
pub fn cq_pending(asid: AddressSpaceId) -> u32 {
    match cq_ring_of(asid) {
        Some(ring_ptr) => unsafe { &*ring_ptr }.pending(),
        None => 0,
    }
}

/// Blocks the calling thread until the CQ ring for `asid` has at least
/// `min_complete` pending entries. This is the kernel-internal implementation
/// of the `wait` syscall (§4.2): the reactor blocks on the CQ, and
/// `complete()` writes entries to the ring + wakes the blocked thread.
///
/// The blocking mechanism uses a simple poll loop with sleep bursts (10 ms)
/// rather than the observer/waker path, because the CQ ring is a ring buffer
/// (not an `Observable`). In the production kernel the ring's head write would
/// fire the waker.
pub fn wait_on_cq(asid: AddressSpaceId, _min_complete: u32) {
    // Simple polling loop — not the final design. In production, the ring
    // head-write would signal an Observable that the blocked thread is
    // registered on, avoiding the sleep loop.
    let tid = SYSTEM_SCHEDULER
        .read()
        .get_lp_scheduler()
        .lock()
        .get_tid();
    if tid.is_none() {
        return;
    }

    loop {
        let pending = cq_pending(asid);
        if pending >= _min_complete {
            return;
        }
        // Busy-wait with a hint. In a real kernel this would be a
        // condition-variable-style block.
        for _ in 0..1000 {
            core::hint::spin_loop();
        }
    }
}
