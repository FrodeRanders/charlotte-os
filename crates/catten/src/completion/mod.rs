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
    klib::observer::{
        Observable,
        Observer,
    },
    memory::AddressSpaceId,
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

/// A capability-free operation (architecture doc §8.4): tracked only by its
/// [`OperationId`], delivered exclusively through the CQ ring, and reclaimed
/// on completion. There is no post-terminal record — a detached operation
/// that has completed no longer exists in the kernel.
struct DetachedOp {
    /// Submitter-chosen correlation token, posted as the CQ entry's cookie.
    user_data: u64,
    /// Reduced lifecycle: `InFlight → CancelPending`; completion removes the
    /// record, so the terminal states of [`OpState`] have no analogue here.
    cancel_pending: bool,
}

struct AsCompletions {
    table: crate::klib::collections::id_table::IdTable<Arc<Completion>>,
    capacity: usize,
    live: usize,
    /// Live capability-free operations, keyed by their stable operation id.
    /// These count toward `capacity` exactly like capability-backed ones.
    detached: BTreeMap<OperationId, DetachedOp>,
    /// Optional per-AS completion-queue ring (zero-syscall drain path).
    /// The allocation backing the ring is kept alive by `_cq_buf`.
    cq_ring: Option<*mut crate::completion::cq::CompletionQueueRing>,
    /// CQ entries (cookie, result) that could not fit in the shared ring yet.
    /// This preserves the non-lossy completion contract: a full userspace ring
    /// delays delivery but does not discard terminal completions.
    pending_cq: VecDeque<(u64, OpResult)>,
    /// Threads blocked waiting for this address space's CQ to become readable.
    cq_observers: ConcurrentQueue<Weak<dyn Observer>>,
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
            detached: BTreeMap::new(),
            cq_ring: None,
            pending_cq: VecDeque::new(),
            cq_observers: ConcurrentQueue::unbounded(),
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
            detached: BTreeMap::new(),
            cq_ring: Some(ring_ptr),
            pending_cq: VecDeque::new(),
            cq_observers: ConcurrentQueue::unbounded(),
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
    let ring_ptr =
        unsafe { crate::completion::cq::CompletionQueueRing::init_at_phys(ring_frame, cq_entries) };
    COMPLETIONS.write().insert(
        asid,
        AsCompletions {
            table: crate::klib::collections::id_table::IdTable::new(),
            capacity: cap_table_capacity,
            live: 0,
            detached: BTreeMap::new(),
            cq_ring: Some(ring_ptr),
            pending_cq: VecDeque::new(),
            cq_observers: ConcurrentQueue::unbounded(),
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
    // to happen-before the wake, not after it.
    {
        let mut registry = COMPLETIONS.write();
        if let Some(as_completions) = registry.get_mut(&asid) {
            flush_pending_cq(as_completions);
            if let Some(ring_ptr) = as_completions.cq_ring {
                if !unsafe { &mut *ring_ptr }.write(cap, effective.clone()) {
                    as_completions.pending_cq.push_back((cap as u64, effective));
                }
            }
        }
    }

    signal_cq(asid);
    completion.signal();

    Ok(())
}

/// Starts a capability-free operation (architecture doc §8.4): the common
/// path for high-rate operations whose only consumer is the CQ ring.
///
/// No capability-table slot is allocated; the operation is identified by the
/// returned [`OperationId`] (for cancellation) and correlated by the caller's
/// `user_data`, which is posted as the CQ entry cookie on completion. The
/// operation counts toward the same submission-backpressure capacity as
/// capability-backed ones. Requires an attached CQ ring, since that is the
/// only delivery channel.
pub fn submit_detached(
    asid: AddressSpaceId,
    _op: OpCode,
    user_data: u64,
) -> Result<OperationId, SubmitError> {
    let mut registry = COMPLETIONS.write();
    let as_completions = registry.get_mut(&asid).ok_or(SubmitError::UnknownAddressSpace)?;
    if as_completions.cq_ring.is_none() {
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
            cancel_pending: false,
        },
    );
    as_completions.live += 1;
    Ok(operation)
}

/// Completes a capability-free operation: posts `(user_data, result)` to the
/// CQ ring (or the non-lossy backlog), reclaims the operation record, and
/// wakes CQ waiters. The effective result is forced to
/// [`OpResult::Cancelled`] when a cancel was pending. After this call the
/// operation id no longer names anything.
pub fn complete_detached(
    asid: AddressSpaceId,
    operation: OperationId,
    result: OpResult,
) -> Result<(), CapError> {
    {
        let mut registry = COMPLETIONS.write();
        let as_completions = registry.get_mut(&asid).ok_or(CapError::UnknownAddressSpace)?;
        let detached = as_completions.detached.remove(&operation).ok_or(CapError::UnknownCap)?;
        let effective = if detached.cancel_pending {
            OpResult::Cancelled
        } else {
            result
        };
        flush_pending_cq(as_completions);
        if let Some(ring_ptr) = as_completions.cq_ring {
            if !unsafe { &mut *ring_ptr }.write_cookie(detached.user_data, effective.clone()) {
                as_completions.pending_cq.push_back((detached.user_data, effective));
            }
        }
        as_completions.live = as_completions.live.saturating_sub(1);
    }
    signal_cq(asid);
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

fn flush_pending_cq(as_completions: &mut AsCompletions) {
    let Some(ring_ptr) = as_completions.cq_ring else {
        return;
    };
    while let Some((cookie, result)) = as_completions.pending_cq.front().cloned() {
        if unsafe { &mut *ring_ptr }.write_cookie(cookie, result) {
            as_completions.pending_cq.pop_front();
        } else {
            break;
        }
    }
}

fn signal_cq(asid: AddressSpaceId) {
    let observers = {
        let registry = COMPLETIONS.read();
        let Some(as_completions) = registry.get(&asid) else {
            return;
        };
        as_completions.cq_observers.try_iter().collect::<Vec<_>>()
    };
    for observer in observers {
        if let Some(observer) = observer.upgrade() {
            observer.notify();
        }
    }
}

struct CqObservable {
    asid: AddressSpaceId,
}

impl Observable for CqObservable {
    fn register_observer(&self, observer: Weak<dyn Observer>) {
        let registry = COMPLETIONS.read();
        if let Some(as_completions) = registry.get(&self.asid) {
            let _ = as_completions.cq_observers.push(observer);
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

/// Polls the CQ ring for `asid` and returns the number of pending entries.
/// Returns 0 if no CQ ring is attached.
pub fn cq_pending(asid: AddressSpaceId) -> u32 {
    let mut registry = COMPLETIONS.write();
    match registry.get_mut(&asid) {
        Some(as_completions) => {
            flush_pending_cq(as_completions);
            match as_completions.cq_ring {
                Some(ring_ptr) => unsafe { &*ring_ptr }.pending(),
                None => 0,
            }
        }
        None => 0,
    }
}

/// Blocks the calling thread until the CQ ring for `asid` has at least
/// `min_complete` pending entries. This is the kernel-internal implementation
/// of the `wait` syscall (§4.2): the reactor blocks on CQ readiness, and
/// `complete()` writes entries to the ring/backlog before waking waiters.
pub fn wait_on_cq(asid: AddressSpaceId, min_complete: u32) {
    let Some(tid) = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid() else {
        return;
    };
    let cq = CqObservable {
        asid,
    };

    loop {
        if cq_pending(asid) >= min_complete {
            return;
        }

        if SYSTEM_SCHEDULER.write().block_thread(tid, &cq).is_err() {
            return;
        }

        // Lost-wake guard: if the CQ became readable while the waker was being
        // registered, re-admit the thread before yielding.
        if cq_pending(asid) >= min_complete {
            let _ = SYSTEM_SCHEDULER.write().submit_ready_thread(tid);
        }

        yield_lp();
    }
}
