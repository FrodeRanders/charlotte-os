//! # Completion-Capability Subsystem (Option C prototype)
//!
//! This module is the first in-kernel prototype of the async syscall /
//! completion-capability ABI specified in `docs/async-syscall-abi.md`. It builds
//! the kernel side of the boundary directly on facilities that already exist:
//!
//! - [`IdTable`](crate::klib::collections::id_table::IdTable) backs a per-address-space
//!   **capability table** mapping a small integer [`CompletionCap`] to a kernel object naming an
//!   in-flight or completed operation;
//! - the [`Observable`]/[`Observer`] mechanism (the same one `TimerEvent` and thread-exit use) is
//!   how a completion signals the threads awaiting it — so [`wait`] blocks exactly as
//!   [`sleep`](crate::cpu::scheduler::sleep) does, registering the caller's `Waker` as an observer;
//! - an owned `Vec<u8>` transferred on [`submit`] is retained by the kernel until a terminal
//!   completion hands it back — the buffer-ownership / deferred-reclaim contract mirrored from
//!   sitas's `io_uring` discipline.
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

use alloc::{
    collections::{
        BTreeMap,
        VecDeque,
    },
    sync::{
        Arc,
        Weak,
    },
    vec::Vec,
};

use concurrent_queue::ConcurrentQueue;
use spin::{
    LazyLock,
    Mutex,
    RwLock,
};

use crate::{
    cpu::scheduler::{
        system_scheduler::SYSTEM_SCHEDULER,
        yield_lp,
    },
    klib::{
        observer::{
            Observable,
            Observer,
        },
        time::duration::ExtDuration,
    },
    memory::AddressSpaceId,
    timers::{
        TIMER_QUEUES,
        TimerEvent,
    },
};

/// A per-address-space handle naming an in-flight or completed async operation.
///
/// This is the kernel realization of the ABI's `Handle` — the value that would
/// cross the syscall boundary. It is an index into the owning address space's
/// capability table. Slot indices are **reused** after [`close`]; see
/// [`OperationId`] for the stable identity of one operation.
pub type CompletionCap = usize;

/// The stable identity of one submitted operation (architecture doc §8.2).
///
/// Unlike [`CompletionCap`] (a reusable table index), an operation id is
/// allocated monotonically and never reused, so completion records remain
/// unambiguous even after capability slots are recycled. This is the
/// identity a capability-free submission path keys on.
pub type OperationId = u64;

static NEXT_OPERATION_ID: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

fn alloc_operation_id() -> OperationId {
    NEXT_OPERATION_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
}

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
    /// Timer completion: auto-completes when a deadline expires.
    Timer,
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

/// The lifecycle state of a submitted operation (architecture doc §12.1).
///
/// A [`Completion`] object exists only after a successful [`submit`] — i.e.
/// after the `Created → Submitted → Accepted` transitions have already
/// succeeded — so the modelled states begin at `InFlight`:
///
/// ```text
/// InFlight ──cancel──▶ CancelPending
///    │                     │
/// complete              complete (forced Cancelled)
///    ▼                     ▼
/// Completed ◀─────────────┘
///    │
///  take (drain result + buffer)
///    ▼
/// Observed
/// ```
///
/// `Completed` and `Observed` are the terminal, reclaimable states. This
/// replaces the previous scattered `cancelling`/`drained` booleans with named
/// states and explicit transitions (architecture doc §18.2).
enum OpState {
    /// Submitted, no terminal result yet.
    InFlight,
    /// Cancellation requested while in flight; the terminal result will be
    /// forced to [`OpResult::Cancelled`].
    CancelPending,
    /// A terminal result has been posted but not yet drained.
    Completed(OpResult),
    /// The terminal result has been drained by [`Completion::take`].
    Observed,
}

/// The externally observable lifecycle state of an operation, for inspection
/// and testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpStateKind {
    InFlight,
    CancelPending,
    Completed,
    Observed,
}

/// Reason a [`submit`] was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum SubmitError {
    /// The capability table is full — first-class, non-fatal backpressure.
    WouldBlock,
    /// No capability table is open for this address space.
    UnknownAddressSpace,
    /// A capability-free ([`submit_detached`]) submission needs a completion
    /// queue to deliver its result, but none is attached to this address space.
    NoCompletionQueue,
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
    /// The operation's lifecycle state; all transitions are made under the
    /// mutex through the methods below (see [`OpState`]).
    state: OpState,
    /// Keeps the exit-observer (if any) alive for as long as the capability
    /// exists. `observe_thread_exit` stores only a `Weak`, so the strong `Arc`
    /// must live somewhere; it lives here, so the observer can still fire when
    /// the worker thread exits.
    exit_observer: Option<Arc<CompletionExitObserver>>,
    /// Keeps the timer observer (if any) alive until the completion is reclaimed.
    timer_observer: Option<Arc<CompletionTimerObserver>>,
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
        let _ = complete(self.asid, self.cap, self.result.clone());
    }
}

/// An [`Observer`] that completes a capability when a timer fires.
/// This is the mechanism backing `OpCode::Timer`: the timer event's
/// observer posts the terminal result, so a userspace caller that
/// blocks on `cq_wait` is released at the deadline.
struct CompletionTimerObserver {
    asid: AddressSpaceId,
    cap: CompletionCap,
    result: OpResult,
}

impl Observer for CompletionTimerObserver {
    fn notify(self: Arc<Self>) {
        let _ = complete(self.asid, self.cap, self.result.clone());
    }
}

/// A kernel object naming one in-flight or completed operation.
///
/// It is [`Observable`]: threads awaiting the operation register their `Waker`
/// here (via [`wait`]), and [`Completion::complete`] notifies them — the exact
/// pattern `TimerEvent` uses for `sleep`.
pub struct Completion {
    /// Stable, never-reused identity of this operation (see [`OperationId`]).
    operation: OperationId,
    inner: Mutex<CompletionInner>,
    observers: ConcurrentQueue<Weak<dyn Observer>>,
}

impl Completion {
    fn new(buffer: Option<Vec<u8>>) -> Arc<Self> {
        Arc::new(Self {
            operation: alloc_operation_id(),
            inner: Mutex::new(CompletionInner {
                buffer,
                state: OpState::InFlight,
                exit_observer: None,
                timer_observer: None,
            }),
            observers: ConcurrentQueue::unbounded(),
        })
    }

    fn operation_id(&self) -> OperationId {
        self.operation
    }

    fn set_exit_observer(&self, observer: Arc<CompletionExitObserver>) {
        self.inner.lock().exit_observer = Some(observer);
    }

    fn set_timer_observer(&self, observer: Arc<CompletionTimerObserver>) {
        self.inner.lock().timer_observer = Some(observer);
    }

    fn state_kind(&self) -> OpStateKind {
        match self.inner.lock().state {
            OpState::InFlight => OpStateKind::InFlight,
            OpState::CancelPending => OpStateKind::CancelPending,
            OpState::Completed(_) => OpStateKind::Completed,
            OpState::Observed => OpStateKind::Observed,
        }
    }

    /// A terminal result has been posted (whether or not it has been drained).
    fn is_terminal(&self) -> bool {
        matches!(self.inner.lock().state, OpState::Completed(_) | OpState::Observed)
    }

    fn is_reclaimable(&self) -> bool {
        self.is_terminal()
    }

    fn holds_buffer(&self) -> bool {
        self.inner.lock().buffer.is_some()
    }

    /// Kernel-side transition: the operation finished. Moves `InFlight` or
    /// `CancelPending` to `Completed`, forcing the result to
    /// [`OpResult::Cancelled`] when a cancel was requested. Returns the
    /// effective terminal result on the first call, or `None` if the operation
    /// was already terminal (idempotent). Does **not** signal observers; the
    /// caller wakes waiters after publishing the CQ entry.
    fn complete(&self, result: OpResult) -> Option<OpResult> {
        let mut inner = self.inner.lock();
        let effective = match inner.state {
            OpState::InFlight => result,
            OpState::CancelPending => OpResult::Cancelled,
            OpState::Completed(_) | OpState::Observed => return None,
        };
        inner.state = OpState::Completed(effective.clone());
        Some(effective)
    }

    fn signal(&self) {
        for observer in self.observers.try_iter() {
            if let Some(observer) = observer.upgrade() {
                observer.notify();
            }
        }
    }

    /// Drains the terminal result and returns the buffer to the caller,
    /// transitioning `Completed → Observed`. Returns `None` while in flight or
    /// once already drained.
    fn take(&self) -> Option<Completed> {
        let mut inner = self.inner.lock();
        match core::mem::replace(&mut inner.state, OpState::Observed) {
            OpState::Completed(result) => {
                let buffer = inner.buffer.take();
                Some(Completed {
                    result,
                    buffer,
                })
            }
            // Restore the prior state: nothing was drained.
            other => {
                inner.state = other;
                None
            }
        }
    }

    /// Requests cancellation. Transitions `InFlight → CancelPending`; if already
    /// terminal, reports it. The buffer is deliberately retained until a
    /// terminal completion is posted (deferred reclaim).
    fn cancel(&self) -> CancelState {
        let mut inner = self.inner.lock();
        match inner.state {
            OpState::InFlight => {
                inner.state = OpState::CancelPending;
                CancelState::CancelRequested
            }
            OpState::CancelPending => CancelState::CancelRequested,
            OpState::Completed(_) | OpState::Observed => CancelState::AlreadyComplete,
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

/// Identifies one completion queue within an address space (architecture doc
/// §8.1: one CQ per shard). Queue 0 ([`DEFAULT_CQ`]) always exists when any
/// CQ is attached and is the destination for capability-backed completions
/// and for callers that do not select a queue.
pub type CqId = u32;

/// The default completion queue of an address space.
pub const DEFAULT_CQ: CqId = 0;

/// A capability-free operation (architecture doc §8.4): tracked only by its
/// [`OperationId`], delivered exclusively through the CQ ring, and reclaimed
/// on completion. There is no post-terminal record — a detached operation
/// that has completed no longer exists in the kernel.
struct DetachedOp {
    /// Submitter-chosen correlation token, posted as the CQ entry's cookie.
    user_data: u64,
    /// The queue this operation's completion is delivered to.
    cq: CqId,
    /// Reduced lifecycle: `InFlight → CancelPending`; completion removes the
    /// record, so the terminal states of [`OpState`] have no analogue here.
    cancel_pending: bool,
}

/// One completion queue: the shared ring plus its non-lossy backlog, pending
/// wake, and blocked waiters. An address space owns one per shard.
struct CqState {
    /// The shared ring (zero-syscall drain path). The allocation backing a
    /// heap-backed ring is kept alive by `_buf`.
    ring: *mut crate::completion::cq::CompletionQueueRing,
    /// Entries (cookie, result) that could not fit in the shared ring yet.
    /// This preserves the non-lossy completion contract: a full userspace
    /// ring delays delivery but does not discard terminal completions.
    backlog: VecDeque<(u64, u64, u32, i64)>,
    /// An explicit cross-thread wake was posted ([`wake`]) and has not yet
    /// been consumed by a waiter on this queue. Consume-on-wait semantics
    /// close the lost-wake race: a wake posted between a waiter's ring check
    /// and its blocking is still observed by the guard re-check.
    wake_pending: bool,
    /// Threads blocked waiting for this queue to become readable.
    observers: ConcurrentQueue<Weak<dyn Observer>>,
    #[allow(dead_code)]
    _buf: Option<alloc::boxed::Box<alloc::vec::Vec<u8>>>,
}

struct AsCompletions {
    table: crate::klib::collections::id_table::IdTable<Arc<Completion>>,
    capacity: usize,
    live: usize,
    /// Live capability-free operations, keyed by their stable operation id.
    /// These count toward `capacity` exactly like capability-backed ones.
    detached: BTreeMap<OperationId, DetachedOp>,
    /// The address space's completion queues, keyed by [`CqId`].
    cqs: BTreeMap<CqId, CqState>,
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

fn empty_as(capacity: usize) -> AsCompletions {
    AsCompletions {
        table: crate::klib::collections::id_table::IdTable::new(),
        capacity,
        live: 0,
        detached: BTreeMap::new(),
        cqs: BTreeMap::new(),
    }
}

/// Opens a bounded capability table for an address space. `capacity` bounds the
/// number of concurrently in-flight capabilities (submission backpressure).
pub fn open_address_space(asid: AddressSpaceId, capacity: usize) {
    COMPLETIONS.write().insert(asid, empty_as(capacity));
}

/// Like [`open_address_space`] but also allocates and attaches the default
/// completion-queue ring ([`DEFAULT_CQ`]). The ring is a single 4 KiB page
/// with `cq_entries` entry slots, accessible from the kernel via the raw
/// pointer stored in the registry.
pub fn open_address_space_with_cq(
    asid: AddressSpaceId,
    cap_table_capacity: usize,
    cq_entries: u32,
) {
    COMPLETIONS.write().insert(asid, empty_as(cap_table_capacity));
    open_cq(asid, DEFAULT_CQ, cq_entries);
}

/// Like [`open_address_space_with_cq`] but initialises the default ring on a
/// pre-allocated physical frame (for mappings where the same frame must also
/// appear in a user page table).
pub fn open_address_space_with_cq_phys(
    asid: AddressSpaceId,
    cap_table_capacity: usize,
    ring_frame: crate::memory::physical::PAddr,
    cq_entries: u32,
) {
    COMPLETIONS.write().insert(asid, empty_as(cap_table_capacity));
    open_cq_phys(asid, DEFAULT_CQ, ring_frame, cq_entries);
}

/// Attaches an additional heap-backed completion queue to an address space —
/// one per shard in the per-shard-CQ model (§8.1). Replaces any existing
/// queue with the same id.
pub fn open_cq(asid: AddressSpaceId, cq: CqId, cq_entries: u32) {
    let (buf, ring_ptr) = crate::completion::cq::CompletionQueueRing::new_page(cq_entries);
    let mut registry = COMPLETIONS.write();
    if let Some(as_completions) = registry.get_mut(&asid) {
        as_completions.cqs.insert(
            cq,
            CqState {
                ring: ring_ptr,
                backlog: VecDeque::new(),
                wake_pending: false,
                observers: ConcurrentQueue::unbounded(),
                _buf: Some(alloc::boxed::Box::new(buf)),
            },
        );
    }
}

/// Attaches an additional completion queue whose ring lives on a
/// pre-allocated physical frame (mappable into the user address space).
pub fn open_cq_phys(
    asid: AddressSpaceId,
    cq: CqId,
    ring_frame: crate::memory::physical::PAddr,
    cq_entries: u32,
) {
    let ring_ptr =
        unsafe { crate::completion::cq::CompletionQueueRing::init_at_phys(ring_frame, cq_entries) };
    let mut registry = COMPLETIONS.write();
    if let Some(as_completions) = registry.get_mut(&asid) {
        as_completions.cqs.insert(
            cq,
            CqState {
                ring: ring_ptr,
                backlog: VecDeque::new(),
                wake_pending: false,
                observers: ConcurrentQueue::unbounded(),
                _buf: None,
            },
        );
    }
}

/// Returns a raw pointer to a CQ ring of `asid`, or `None`.
pub fn cq_ring_of(
    asid: AddressSpaceId,
    cq: CqId,
) -> Option<*mut crate::completion::cq::CompletionQueueRing> {
    let registry = COMPLETIONS.read();
    registry.get(&asid).and_then(|c| c.cqs.get(&cq)).map(|state| state.ring)
}

/// Closes an address space's capability table and frees its CQ ring (if any).
pub fn close_address_space(asid: AddressSpaceId) {
    COMPLETIONS.write().remove(&asid);
}

pub fn completion_of(
    asid: AddressSpaceId,
    cap: CompletionCap,
) -> Result<Arc<Completion>, CapError> {
    let registry = COMPLETIONS.read();
    let as_completions = registry.get(&asid).ok_or(CapError::UnknownAddressSpace)?;
    let completion = as_completions.table.get(cap).map_err(|_| CapError::UnknownCap)?;
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
    let as_completions = registry.get_mut(&asid).ok_or(SubmitError::UnknownAddressSpace)?;
    if as_completions.live >= as_completions.capacity {
        return Err(SubmitError::WouldBlock);
    }
    let cap = as_completions.table.add_element(Completion::new(buffer));
    as_completions.live += 1;
    Ok(cap)
}

/// Submits a timer operation: creates a capability that auto-completes after
/// `timeout_ms` milliseconds. The returned cap delivers a completion ring entry
/// when the deadline expires, so a user-space service waiting on `cq_wait` is
/// released exactly at the deadline.
pub fn submit_timer(
    asid: AddressSpaceId,
    timeout_ms: u64,
) -> Result<CompletionCap, SubmitError> {
    let cap = submit(asid, OpCode::Timer, None)?;
    let observer = Arc::new(CompletionTimerObserver {
        asid,
        cap,
        result: OpResult::Ok(0),
    });
    let timer_event = TimerEvent::from(ExtDuration::from_millis(timeout_ms as u128));
    timer_event.register_observer(Arc::downgrade(&observer) as Weak<dyn Observer>);
    unsafe { TIMER_QUEUES.try_get_mut().unwrap_unchecked() }.add_event(timer_event);
    if let Ok(completion) = completion_of(asid, cap) {
        completion.set_timer_observer(observer);
    }
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
    let observer = Arc::new(CompletionExitObserver {
        asid,
        cap,
        result,
    });
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
/// finished. Transitions the operation to `Completed`, publishes the entry to
/// the AS's CQ ring (if attached), and wakes awaiting threads.
///
/// The **effective** terminal result — [`OpResult::Cancelled`] when a cancel
/// was pending — is what reaches both the capability and the CQ ring, so the
/// two views can never disagree. Idempotent: a second completion neither
/// changes the result nor posts a duplicate CQ entry.
pub fn complete(
    asid: AddressSpaceId,
    cap: CompletionCap,
    result: OpResult,
) -> Result<(), CapError> {
    let completion = completion_of(asid, cap)?;

    // Transition first so the CQ entry carries the effective terminal result.
    let Some(effective) = completion.complete(result) else {
        // Already terminal: idempotent no-op, no duplicate CQ entry.
        return Ok(());
    };

    // Publish the completion entry to the shared CQ ring *before* waking any
    // waiter. A userspace consumer that blocks in `wait` and then drains the
    // ring the moment it is woken must observe the entry, so the ring write has
    // to happen-before the wake, not after it. Capability-backed completions
    // are delivered to the default queue.
    {
        let mut registry = COMPLETIONS.write();
        if let Some(as_completions) = registry.get_mut(&asid) {
            if let Some(cq_state) = as_completions.cqs.get_mut(&DEFAULT_CQ) {
                let op = completion.operation_id();
                post_to_cq(cq_state, op, cap as u64, &effective);
            }
        }
    }

    signal_cq(asid, DEFAULT_CQ);
    completion.signal();

    Ok(())
}

/// Posts one entry to a queue's ring, spilling to its non-lossy backlog when
/// the ring is full. Any backlog is flushed (batched) first so ordering is
/// preserved.
fn post_to_cq(cq_state: &mut CqState, operation: u64, cookie: u64, result: &OpResult) {
    let (status, val) = crate::completion::cq::op_result_to_fields(result);
    flush_backlog(cq_state);
    if !cq_state.backlog.is_empty()
        || !unsafe { &mut *cq_state.ring }.write(operation, cookie, status, val)
    {
        cq_state.backlog.push_back((operation, cookie, status, val));
    }
}

/// Starts a capability-free operation (architecture doc §8.4): the common
/// path for high-rate operations whose only consumer is the CQ ring.
///
/// No capability-table slot is allocated; the operation is identified by the
/// returned [`OperationId`] (for cancellation) and correlated by the caller's
/// `user_data`, which is posted as the CQ entry cookie on completion to the
/// selected queue `cq` (per-shard delivery, §8.1). The operation counts
/// toward the same submission-backpressure capacity as capability-backed
/// ones. Requires the selected queue to exist, since it is the only delivery
/// channel.
pub fn submit_detached(
    asid: AddressSpaceId,
    cq: CqId,
    _op: OpCode,
    user_data: u64,
) -> Result<OperationId, SubmitError> {
    let mut registry = COMPLETIONS.write();
    let as_completions = registry.get_mut(&asid).ok_or(SubmitError::UnknownAddressSpace)?;
    if !as_completions.cqs.contains_key(&cq) {
        return Err(SubmitError::NoCompletionQueue);
    }
    if as_completions.live >= as_completions.capacity {
        return Err(SubmitError::WouldBlock);
    }
    let operation = alloc_operation_id();
    as_completions.detached.insert(
        operation,
        DetachedOp {
            user_data,
            cq,
            cancel_pending: false,
        },
    );
    as_completions.live += 1;
    Ok(operation)
}

/// Completes a capability-free operation: posts `(user_data, result)` to the
/// operation's queue (or its non-lossy backlog), reclaims the operation
/// record, and wakes that queue's waiters. The effective result is forced to
/// [`OpResult::Cancelled`] when a cancel was pending. After this call the
/// operation id no longer names anything.
pub fn complete_detached(
    asid: AddressSpaceId,
    operation: OperationId,
    result: OpResult,
) -> Result<(), CapError> {
    let cq = {
        let mut registry = COMPLETIONS.write();
        let as_completions = registry.get_mut(&asid).ok_or(CapError::UnknownAddressSpace)?;
        let detached = as_completions.detached.remove(&operation).ok_or(CapError::UnknownCap)?;
        let effective = if detached.cancel_pending {
            OpResult::Cancelled
        } else {
            result
        };
        if let Some(cq_state) = as_completions.cqs.get_mut(&detached.cq) {
            post_to_cq(cq_state, operation, detached.user_data, &effective);
        }
        as_completions.live = as_completions.live.saturating_sub(1);
        detached.cq
    };
    signal_cq(asid, cq);
    Ok(())
}

/// Requests cancellation of a capability-free operation. A completed detached
/// operation no longer exists, so cancelling it reports
/// [`CapError::UnknownCap`] rather than `AlreadyComplete` — there is no
/// post-terminal record to inspect.
pub fn cancel_detached(
    asid: AddressSpaceId,
    operation: OperationId,
) -> Result<CancelState, CapError> {
    let mut registry = COMPLETIONS.write();
    let as_completions = registry.get_mut(&asid).ok_or(CapError::UnknownAddressSpace)?;
    let detached = as_completions.detached.get_mut(&operation).ok_or(CapError::UnknownCap)?;
    detached.cancel_pending = true;
    Ok(CancelState::CancelRequested)
}

/// Batched backlog flush: writes as many retained entries as fit with one
/// ring head update, preserving order.
fn flush_backlog(cq_state: &mut CqState) {
    if cq_state.backlog.is_empty() {
        return;
    }
    let entries = cq_state.backlog.make_contiguous();
    let written = unsafe { &mut *cq_state.ring }.write_batch(entries.iter());
    let _ = entries;
    for _ in 0..written {
        cq_state.backlog.pop_front();
    }
}

fn signal_cq(asid: AddressSpaceId, cq: CqId) {
    let observers = {
        let registry = COMPLETIONS.read();
        let Some(cq_state) = registry.get(&asid).and_then(|c| c.cqs.get(&cq)) else {
            return;
        };
        cq_state.observers.try_iter().collect::<Vec<_>>()
    };
    for observer in observers {
        if let Some(observer) = observer.upgrade() {
            observer.notify();
        }
    }
}

struct CqObservable {
    asid: AddressSpaceId,
    cq: CqId,
}

impl Observable for CqObservable {
    fn register_observer(&self, observer: Weak<dyn Observer>) {
        let registry = COMPLETIONS.read();
        if let Some(cq_state) = registry.get(&self.asid).and_then(|c| c.cqs.get(&self.cq)) {
            let _ = cq_state.observers.push(observer);
        }
    }
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
    if completion.is_terminal() {
        return Ok(());
    }

    let tid =
        SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid().ok_or(CapError::UnknownCap)?;

    SYSTEM_SCHEDULER
        .write()
        .block_thread(tid, completion.as_ref() as &dyn Observable)
        .map_err(|_| CapError::UnknownCap)?;

    // Lost-wake guard: if the operation completed after our fast-path check but
    // before (or during) registration, make the thread runnable again.
    if completion.is_terminal() {
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
    as_completions.table.remove_element(cap).map_err(|_| CapError::UnknownCap)?;
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

/// Returns the stable [`OperationId`] of the operation named by `cap`.
pub fn operation_id(asid: AddressSpaceId, cap: CompletionCap) -> Result<OperationId, CapError> {
    Ok(completion_of(asid, cap)?.operation_id())
}

/// Inspection: the current lifecycle state of the operation named by `cap`.
pub fn state_of(asid: AddressSpaceId, cap: CompletionCap) -> Result<OpStateKind, CapError> {
    Ok(completion_of(asid, cap)?.state_kind())
}

/// Polls a CQ ring of `asid` and returns the number of pending entries
/// (flushing the non-lossy backlog first). Returns 0 if the queue does not
/// exist.
pub fn cq_pending(asid: AddressSpaceId, cq: CqId) -> u32 {
    let mut registry = COMPLETIONS.write();
    match registry.get_mut(&asid).and_then(|c| c.cqs.get_mut(&cq)) {
        Some(cq_state) => {
            flush_backlog(cq_state);
            unsafe { &*cq_state.ring }.pending()
        }
        None => 0,
    }
}

/// Posts an explicit wake to the waiters of one queue (architecture doc
/// §7.3/§9.4): a thread blocked in [`wait_on_cq`]/[`wait_on_cq_timeout`] on
/// that queue returns even though no completion entry was posted. Used by
/// userspace reactors so a peer shard can interrupt a blocking CQ wait (for
/// example when new internal work is queued). Wakes are consume-on-wait and
/// coalesce: any number of wakes before the next wait release exactly one
/// waiter pass.
pub fn wake(asid: AddressSpaceId, cq: CqId) {
    {
        let mut registry = COMPLETIONS.write();
        if let Some(cq_state) = registry.get_mut(&asid).and_then(|c| c.cqs.get_mut(&cq)) {
            cq_state.wake_pending = true;
        }
    }
    signal_cq(asid, cq);
}

/// Consumes a pending wake, if any.
fn take_wake(asid: AddressSpaceId, cq: CqId) -> bool {
    let mut registry = COMPLETIONS.write();
    match registry.get_mut(&asid).and_then(|c| c.cqs.get_mut(&cq)) {
        Some(cq_state) => core::mem::take(&mut cq_state.wake_pending),
        None => false,
    }
}

/// Non-consuming check for a pending wake (lost-wake guard re-check).
fn peek_wake(asid: AddressSpaceId, cq: CqId) -> bool {
    let registry = COMPLETIONS.read();
    registry
        .get(&asid)
        .and_then(|c| c.cqs.get(&cq))
        .map(|state| state.wake_pending)
        .unwrap_or(false)
}

/// Blocks the calling thread until queue `cq` of `asid` has at least
/// `min_complete` pending entries **or** an explicit [`wake`] is posted to
/// that queue. This is the kernel-internal implementation of the `CQ_WAIT`
/// syscall (§4.2): the reactor blocks on CQ readiness, and `complete()`
/// writes entries to the ring/backlog before waking waiters.
pub fn wait_on_cq(asid: AddressSpaceId, cq: CqId, min_complete: u32) {
    let Some(tid) = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid() else {
        return;
    };
    let observable = CqObservable {
        asid,
        cq,
    };

    loop {
        if take_wake(asid, cq) || cq_pending(asid, cq) >= min_complete {
            return;
        }

        if SYSTEM_SCHEDULER.write().block_thread(tid, &observable).is_err() {
            return;
        }

        // Lost-wake guard: if the CQ became readable (or a wake was posted)
        // while the waker was being registered, re-admit the thread before
        // yielding.
        if peek_wake(asid, cq) || cq_pending(asid, cq) >= min_complete {
            let _ = SYSTEM_SCHEDULER.write().submit_ready_thread(tid);
        }

        yield_lp();
    }
}

/// Like [`wait_on_cq`] but also returns when `timeout_ms` elapses. Returns
/// whether the CQ readiness/wake condition was met (`true`) or the deadline
/// fired first (`false`).
pub fn wait_on_cq_timeout(
    asid: AddressSpaceId,
    cq: CqId,
    min_complete: u32,
    timeout_ms: u64,
) -> bool {
    use crate::{
        klib::time::duration::ExtDuration,
        timers::{
            TIMER_QUEUES,
            TimerEvent,
        },
    };

    struct CqTimeoutWake {
        tid: crate::cpu::scheduler::threads::ThreadId,
    }
    impl Observer for CqTimeoutWake {
        fn notify(self: Arc<Self>) {
            let _ = SYSTEM_SCHEDULER.read().submit_ready_thread(self.tid);
        }
    }

    let Some(tid) = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid() else {
        return false;
    };

    if take_wake(asid, cq) || cq_pending(asid, cq) >= min_complete {
        return true;
    }

    let observable = CqObservable {
        asid,
        cq,
    };
    if SYSTEM_SCHEDULER.write().block_thread(tid, &observable).is_err() {
        return false;
    }

    // Arm a timer that also wakes this thread (timeout path). The observer
    // must outlive the sleep, so keep the strong Arc on this stack frame.
    let timeout_obs = Arc::new(CqTimeoutWake {
        tid,
    });
    let timer_event = TimerEvent::from(ExtDuration::from_millis(timeout_ms as u128));
    crate::klib::observer::Observable::register_observer(
        &timer_event,
        Arc::downgrade(&timeout_obs) as Weak<dyn Observer>,
    );
    // SAFETY: TIMER_QUEUES is initialised by bsp_init before self-tests or
    // any threads run.
    unsafe { TIMER_QUEUES.try_get_mut().unwrap_unchecked() }.add_event(timer_event);

    // Lost-wake guard.
    if peek_wake(asid, cq) || cq_pending(asid, cq) >= min_complete {
        let _ = SYSTEM_SCHEDULER.write().submit_ready_thread(tid);
    }

    yield_lp();

    // Report whether the condition (rather than the deadline) released us,
    // consuming a wake if one was posted.
    take_wake(asid, cq) || cq_pending(asid, cq) >= min_complete
}
