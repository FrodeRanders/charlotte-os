//! # Completion-Capability Subsystem (Option C prototype)
//!
//! This module is the first in-kernel prototype of the async syscall /
//! completion-capability ABI specified in `docs/architecture/async-syscall-abi.md`. It builds
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

pub(crate) mod budget;
pub(crate) mod callback_tests;
pub mod cq;
pub(crate) mod cq_budget;
pub(crate) mod exit_tests;
pub(crate) mod watch_budget;

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

use spin::LazyLock;

use crate::{
    cpu::{
        multiprocessor::spin::{
            mutex::Mutex,
            rwlock::RwLock,
        },
        scheduler::{
            system_scheduler::SYSTEM_SCHEDULER,
            yield_lp,
        },
    },
    klib::{
        charged_allocator::ChargedAllocator,
        observer::{
            Observable,
            Observer,
        },
        time::duration::ExtDuration,
    },
    memory::AddressSpaceId,
    timers::TimerEvent,
};

/// A per-address-space handle naming an in-flight or completed async operation.
///
/// This is the kernel realization of the ABI's `Handle` — the value that would
/// cross the syscall boundary. It is an index into the owning address space's
/// unified object-capability table. Values are never reused within an address
/// space, so a stale handle cannot alias a later operation.
pub type CompletionCap = u64;

// Kernel-private owners carry record admission through all strong/weak handles.
pub(crate) type CompletionRef = Arc<Completion, ChargedAllocator<budget::Charge>>;
pub(crate) type CompletionWeak = Weak<Completion, ChargedAllocator<budget::Charge>>;

/// The stable identity of one submitted operation (architecture doc §8.2).
///
/// Operation ids are independent of capability handles and also identify
/// capability-free submissions. They remain the identity recorded in
/// completion queues even when no first-class capability was requested.
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
    /// Worker cancellation remains deferred until its producer exits. Unlike
    /// an external join, cancelling it must not revoke the terminal event.
    worker_observation: Option<EventObservation>,
    /// Keeps the timer observer (if any) alive until the completion is reclaimed.
    timer_observer: Option<Arc<CompletionTimerObserver>>,
    timer_cancel: Option<TimerCancellation>,
    /// Keeps a subsystem-defined event observer alive. IPC uses this for
    /// connection endpoint-close watches without coupling completion storage
    /// to the concrete IPC observer type.
    event_observation: Option<EventObservation>,
    /// Non-scheduler callbacks use a separate lazy one-shot list. Its entries
    /// share event-watch admission, not scheduler-waiter sponsorship.
    callbacks: Option<crate::klib::observer::registration::ListRef<watch_budget::Charge>>,
}

struct EventObservation {
    _observer: Arc<dyn Observer>,
    _registration: crate::klib::observer::registration::Registration<watch_budget::Charge>,
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
    completion: CompletionWeak,
}

impl Observer for CompletionExitObserver {
    fn notify(self: Arc<Self>) {
        if let Some(completion) = self.completion.upgrade() {
            let _ = complete_registered(self.asid, self.cap, completion, self.result.clone());
        }
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
    completion: CompletionWeak,
}

impl Observer for CompletionTimerObserver {
    fn notify(self: Arc<Self>) {
        if let Some(completion) = self.completion.upgrade() {
            let _ = complete_registered(self.asid, self.cap, completion, self.result.clone());
        }
    }
}

/// A kernel-boundary owner for a completion's anonymous timer. Releasing a
/// record cancels its event, but the event retains its admission charge until
/// its owning LP actually removes the queue node.
struct TimerCancellation(Option<crate::timers::TimerEventCancelHandle>);

impl Drop for TimerCancellation {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            let _ = crate::timers::cancel_event(handle);
        }
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
    waiters:
        crate::klib::observer::registration::ListRef<crate::klib::observer::waiter_budget::Charge>,
}

impl Completion {
    fn new(
        buffer: Option<Vec<u8>>,
        record_charge: budget::Charge,
    ) -> Result<CompletionRef, SubmitError> {
        let platform = record_charge.platform();
        let allocator =
            ChargedAllocator::try_new(record_charge).map_err(|_| SubmitError::WouldBlock)?;
        let waiters = crate::klib::observer::registration::ObserverList::try_new(
            crate::klib::observer::waiter_budget::SOURCE_LIMIT,
            platform,
        )
        .map_err(|_| SubmitError::WouldBlock)?;
        allocator
            .try_arc(Self {
                operation: alloc_operation_id(),
                inner: Mutex::new(CompletionInner {
                    buffer,
                    state: OpState::InFlight,
                    worker_observation: None,
                    timer_observer: None,
                    timer_cancel: None,
                    event_observation: None,
                    callbacks: None,
                }),
                waiters,
            })
            .map_err(|_| SubmitError::WouldBlock)
    }

    fn operation_id(&self) -> OperationId {
        self.operation
    }

    fn set_worker_observation(&self, observation: EventObservation) {
        let mut inner = self.inner.lock();
        if matches!(inner.state, OpState::InFlight | OpState::CancelPending) {
            inner.worker_observation = Some(observation);
        }
    }

    fn set_timer_observer(
        &self,
        observer: Arc<CompletionTimerObserver>,
        cancel: crate::timers::TimerEventCancelHandle,
    ) {
        let mut inner = self.inner.lock();
        inner.timer_observer = Some(observer);
        inner.timer_cancel = Some(TimerCancellation(Some(cancel)));
    }

    /// Returns whether cancellation won before this owner could be installed.
    /// A terminal completion must not retain a late registration.
    pub(crate) fn set_event_observation(
        &self,
        observer: Arc<dyn Observer>,
        registration: crate::klib::observer::registration::Registration<watch_budget::Charge>,
    ) -> bool {
        let observation = EventObservation {
            _observer: observer,
            _registration: registration,
        };
        let mut inner = self.inner.lock();
        match inner.state {
            OpState::InFlight => {
                inner.event_observation = Some(observation);
                false
            }
            OpState::CancelPending => true,
            OpState::Completed(_) | OpState::Observed => false,
        }
    }

    fn release_event_observation(&self) {
        let observations = {
            let mut inner = self.inner.lock();
            (inner.event_observation.take(), inner.worker_observation.take())
        };
        drop(observations);
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
    pub(crate) fn is_terminal(&self) -> bool {
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
        let event = inner.event_observation.take();
        let worker = inner.worker_observation.take();
        drop(inner);
        drop((event, worker));
        Some(effective)
    }

    fn signal(&self) {
        self.waiters.close().notify();
        self.detach_callbacks().notify();
    }

    fn detach_callbacks(
        &self,
    ) -> crate::klib::observer::registration::NotificationBatch<watch_budget::Charge> {
        let callbacks = self.inner.lock().callbacks.take();
        callbacks
            .map_or_else(crate::klib::observer::registration::NotificationBatch::empty, |list| {
                list.close()
            })
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
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &crate::klib::observer::WaitSponsor,
    ) -> Result<
        crate::klib::observer::WaitRegistration,
        crate::klib::observer::registration::RegistrationError,
    > {
        if self.is_terminal() {
            return Ok(crate::klib::observer::WaitRegistration::ready());
        }
        match sponsor.register(&self.waiters, observer) {
            Err(crate::klib::observer::registration::RegistrationError::Closed)
                if self.is_terminal() =>
            {
                Ok(crate::klib::observer::WaitRegistration::ready())
            }
            result => result,
        }
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        // Retained tokens must not keep source entries after the operation is
        // destroyed. Destruction discards callbacks, never invents completion.
        drop(self.waiters.close());
        drop(self.detach_callbacks());
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
/// on completion. The operation ID then stops being addressable, but an
/// undelivered CQ result retains record admission until publication/discard.
struct DetachedOp {
    /// Submitter-chosen correlation token, posted as the CQ entry's cookie.
    user_data: u64,
    /// The queue this operation's completion is delivered to.
    cq: CqId,
    /// Reduced lifecycle: `InFlight → CancelPending`; completion removes the
    /// record, so the terminal states of [`OpState`] have no analogue here.
    cancel_pending: bool,
    /// Keeps a timer observer alive until it fires and removes this operation.
    _timer_observer: Option<Arc<DetachedTimerObserver>>,
    timer_cancel: Option<TimerCancellation>,
    record_charge: budget::Charge,
}

struct DetachedTimerObserver {
    asid: AddressSpaceId,
    operation: OperationId,
}

#[derive(Debug)]
enum BacklogOwner {
    Capability,
    Detached {
        _charge: budget::Charge,
    },
}

#[derive(Debug)]
struct BacklogEntry {
    operation: OperationId,
    cookie: u64,
    status: u32,
    result: i64,
    owner: BacklogOwner,
}

impl Observer for DetachedTimerObserver {
    fn notify(self: Arc<Self>) {
        let _ = complete_detached(self.asid, self.operation, OpResult::Ok(0));
    }
}

/// One completion queue: the shared ring plus its bounded non-lossy backlog
/// and a monotonic work-generation counter. An address space owns one per
/// shard.
struct CqState {
    /// The shared ring (zero-syscall drain path). The allocation backing a
    /// heap-backed ring is kept alive by `_buf`.
    ring: *mut crate::completion::cq::CompletionQueueRing,
    /// Kernel-authoritative producer cursor and capacity. The ring page is
    /// mapped writable to EL0, so `head`/`capacity` in shared memory are only
    /// a publication mirror; the producer must never read them back.
    ring_head: u32,
    ring_capacity: u32,
    /// Entries that could not fit in the shared ring yet. Every entry either
    /// belongs to a live capability or retains a detached operation's live
    /// submission slot, so the address-space capacity also bounds this queue.
    /// Closing a cap-based completion removes its undelivered redundant entry.
    backlog: VecDeque<BacklogEntry>,
    retained_limit: usize,
    /// Monotonic counter bumped every time new work arrives on this queue:
    /// a completion is posted or an explicit wake is posted.  Used by
    /// [`wait_on_cq`] to detect new work without depending on the shared
    /// ring's `pending()` count — callers that poll individual completions
    /// via `poll(cap)` and never drain the ring are not stuck in a busy-spin.
    work_generation: u64,
    /// The `work_generation` value consumed by this queue's reactor. Only new
    /// work releases the next wait. Charlotte's per-shard model has one
    /// blocking reactor per CQ; multiple independent waiters would require a
    /// cursor per waiter rather than this queue-wide cursor.
    last_seen_generation: u64,
    /// Threads blocked waiting for this queue to become readable.
    waiters:
        crate::klib::observer::registration::ListRef<crate::klib::observer::waiter_budget::Charge>,
    #[allow(dead_code)]
    _buf: Option<alloc::vec::Vec<u64>>,
    // Last: free heap backing before returning admission.
    _storage_charge: cq_budget::Charge,
}

impl CqState {
    /// Sanitized consumer cursor. EL0 owns `tail` but may hand back garbage;
    /// an out-of-range value is treated as empty so producer arithmetic stays
    /// bounded by the kernel-side capacity.
    fn shared_tail(&self) -> u32 {
        let tail = unsafe { core::ptr::read_volatile(&(*self.ring).tail) };
        if tail < self.ring_capacity {
            tail
        } else {
            self.ring_head
        }
    }

    fn ring_has_space(&self) -> bool {
        (self.ring_head + 1) % self.ring_capacity != self.shared_tail()
    }

    fn ring_pending(&self) -> u32 {
        let tail = self.shared_tail();
        if self.ring_head >= tail {
            self.ring_head - tail
        } else {
            self.ring_head + self.ring_capacity - tail
        }
    }

    /// Publishes one entry to the shared ring using the kernel-authoritative
    /// producer cursor. Returns `false` when the ring is full.
    fn push_completion(&mut self, operation: u64, cookie: u64, status: u32, result: i64) -> bool {
        if !self.ring_has_space() {
            let overflow = unsafe { core::ptr::read_volatile(&(*self.ring).overflow) };
            unsafe {
                core::ptr::write_volatile(&mut (*self.ring).overflow, overflow.wrapping_add(1));
            }
            return false;
        }
        let head = self.ring_head;
        let entry = unsafe { &mut *(*self.ring).entry_ptr(head as usize) };
        unsafe {
            core::ptr::write_volatile(&mut entry.operation, operation);
            core::ptr::write_volatile(&mut entry.cookie, cookie);
            core::ptr::write_volatile(&mut entry.status, status);
            core::ptr::write_volatile(&mut entry.flags, 0u32);
            core::ptr::write_volatile(&mut entry.result, result);
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        let next = (head + 1) % self.ring_capacity;
        unsafe {
            core::ptr::write_volatile(&mut (*self.ring).head, next);
        }
        self.ring_head = next;
        true
    }
}

impl Drop for CqState {
    fn drop(&mut self) {
        drop(self.waiters.close());
    }
}
struct AsCompletions {
    table: BTreeMap<CompletionCap, CompletionRef>,
    /// Shared upper bound for capability records, in-flight detached
    /// operations, and completed detached records awaiting CQ delivery.
    capacity: usize,
    live: usize,
    /// Live capability-free operations, keyed by their stable operation id.
    /// These count toward `capacity` exactly like capability-backed ones.
    detached: BTreeMap<OperationId, DetachedOp>,
    /// The address space's completion queues, keyed by [`CqId`].
    cqs: BTreeMap<CqId, CqState>,
    /// Separate lifetime budget: a cancelled timer on another LP stays charged
    /// even after its completion slot has been reclaimed.
    timer_budget: Arc<crate::timers::budget::DomainBudget>,
    record_budget: Arc<budget::DomainBudget>,
    cq_budget: Arc<cq_budget::DomainBudget>,
    watch_budget: Arc<watch_budget::DomainBudget>,
    address_space: Option<crate::memory::AddressSpaceHandle>,
}

impl Drop for AsCompletions {
    fn drop(&mut self) {
        // Public watches are revoked even if a kernel waiter retains their
        // completion object. Only independent list locks are entered here.
        for completion in self.table.values() {
            completion.release_event_observation();
            drop(completion.waiters.close());
            drop(completion.detach_callbacks());
        }
    }
}

// SAFETY: `CqState::ring` has two backing modes. Heap-backed queues retain their
// aligned allocation in `_buf: Some(Vec<u64>)`. Physical queues use `_buf: None`; their
// frame is owned by the user address-space mapping, and address-space teardown
// calls `completion::close_address_space` before removing that mapping/table.
// Consequently either backing allocation outlives its registry entry.
//
// All safe kernel dereferences of `ring` and mutations of CQ producer state are
// serialized by the `COMPLETIONS` write lock. The mapped EL0 consumer may update
// the ring tail concurrently; the ring implementation uses volatile accesses
// and acquire/release fences for that producer/consumer protocol. The only raw
// pointer escape, `cq_ring_of`, is unsafe and places its quiescence/lifetime
// obligation on its test/inspection caller. These invariants make moving or
// sharing the registry between kernel threads sound.
unsafe impl Send for AsCompletions {}
unsafe impl Sync for AsCompletions {}

/// Per-address-space capability tables. In a full design these would live inside
/// each `AddressSpace`; keying them here keeps the prototype self-contained
/// without modifying the address-space type.
static COMPLETIONS: LazyLock<RwLock<BTreeMap<AddressSpaceId, AsCompletions>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));

fn empty_as(asid: AddressSpaceId, capacity: usize) -> AsCompletions {
    let capacity = capacity.min(budget::MAX_DOMAIN_RECORDS);
    AsCompletions {
        table: BTreeMap::new(),
        capacity,
        live: 0,
        detached: BTreeMap::new(),
        cqs: BTreeMap::new(),
        timer_budget: crate::timers::budget::DomainBudget::new(capacity),
        record_budget: budget::DomainBudget::new(capacity),
        cq_budget: cq_budget::DomainBudget::new(),
        watch_budget: watch_budget::DomainBudget::new(capacity),
        address_space: crate::memory::current_address_space_handle(asid),
    }
}

/// Budget guards never enter another subsystem. A captured designation must
/// match this namespace, and retirement fences admission under the registry.
fn reserve_record(
    asid: AddressSpaceId,
    entries: &AsCompletions,
    platform_identity: Option<crate::memory::AddressSpaceHandle>,
) -> Result<budget::Charge, SubmitError> {
    if entries.address_space.is_some_and(|handle| !crate::memory::budget::accepting(handle)) {
        return Err(SubmitError::UnknownAddressSpace);
    }
    // Synthetic namespaces exist at the kernel test/adapter boundary only.
    // Syscalls supply the authenticated caller, never an EL0-selected ASID.
    let platform = asid == crate::memory::KERNEL_ASID
        || platform_identity.is_some_and(|handle| Some(handle) == entries.address_space);
    budget::reserve(&entries.record_budget, platform).map_err(|_| SubmitError::WouldBlock)
}

fn replace_address_space(asid: AddressSpaceId, replacement: AsCompletions) {
    if let Some(previous) = COMPLETIONS.write().insert(asid, replacement) {
        for cap in previous.table.keys() {
            assert!(
                crate::capability::remove(asid, *cap, crate::capability::ObjectKind::Completion),
                "completion payload capability was absent from unified table"
            );
        }
    }
}

/// Opens a bounded capability table for an address space. `capacity` bounds the
/// number of concurrently in-flight capabilities (submission backpressure).
pub fn open_address_space(asid: AddressSpaceId, capacity: usize) {
    replace_address_space(asid, empty_as(asid, capacity));
}

/// Like [`open_address_space`] but also allocates and attaches the default
/// completion-queue ring ([`DEFAULT_CQ`]). The ring is a single 4 KiB page
/// with `cq_entries` entry slots, accessible from the kernel via the raw
/// pointer stored in the registry.
pub fn open_address_space_with_cq(
    asid: AddressSpaceId,
    cap_table_capacity: usize,
    cq_entries: u32,
) -> Result<(), CqOpenError> {
    open_namespace_with_cq(asid, cap_table_capacity, cq_entries, None)
}

/// Like [`open_address_space_with_cq`] but initialises the default ring on a
/// pre-allocated physical frame (for mappings where the same frame must also
/// appear in a user page table).
pub fn open_address_space_with_cq_phys(
    asid: AddressSpaceId,
    cap_table_capacity: usize,
    ring_frame: crate::memory::physical::PAddr,
    cq_entries: u32,
) -> Result<(), CqOpenError> {
    open_namespace_with_cq(asid, cap_table_capacity, cq_entries, Some(ring_frame))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CqOpenError {
    UnknownAddressSpace,
    RetiringAddressSpace,
    ResourceLimit,
    AllocationFailed,
    RingInUse,
    Ring(cq::CqRingError),
}

fn check_ring_alias(
    registry: &BTreeMap<AddressSpaceId, AsCompletions>,
    asid: AddressSpaceId,
    cq: CqId,
    frame: Option<crate::memory::physical::PAddr>,
) -> Result<(), CqOpenError> {
    if let Some(frame) = frame {
        let ptr: *mut cq::CompletionQueueRing = frame.into();
        if registry.iter().any(|(owner, namespace)| {
            namespace
                .cqs
                .iter()
                .any(|(id, state)| state.ring == ptr && (*owner != asid || *id != cq))
        }) {
            return Err(CqOpenError::RingInUse);
        }
    }
    Ok(())
}

/// Stage admitted backing before initializing a physical ring or publication.
/// A kernel caller of the physical variant must keep its mapped frame live
/// and quiesce any consumer before replacing that ring.
fn stage_cq(
    asid: AddressSpaceId,
    entries: &AsCompletions,
    cq_entries: u32,
    platform_identity: Option<crate::memory::AddressSpaceHandle>,
    frame: Option<crate::memory::physical::PAddr>,
) -> Result<CqState, CqOpenError> {
    if cq_entries < 2 {
        return Err(CqOpenError::Ring(cq::CqRingError::CapacityTooSmall));
    }
    if entries.address_space.is_some_and(|handle| !crate::memory::budget::accepting(handle)) {
        return Err(CqOpenError::RetiringAddressSpace);
    }
    let slots = charlotte_lifecycle::resources::queue_backing_slots(entries.capacity)
        .ok_or(CqOpenError::ResourceLimit)?;
    let bytes = slots
        .checked_mul(core::mem::size_of::<BacklogEntry>())
        .and_then(|bytes| {
            bytes.checked_add(
                if frame.is_none() {
                    4096
                } else {
                    0
                },
            )
        })
        .ok_or(CqOpenError::ResourceLimit)?;
    let platform = asid == crate::memory::KERNEL_ASID
        || platform_identity.is_some_and(|handle| Some(handle) == entries.address_space);
    let charge = cq_budget::reserve(&entries.cq_budget, platform, [1, bytes as u64])
        .map_err(|_| CqOpenError::ResourceLimit)?;
    let mut state = CqState {
        ring: core::ptr::null_mut(),
        ring_head: 0,
        ring_capacity: cq::CompletionQueueRing::capacity_for(cq_entries),
        backlog: VecDeque::new(),
        retained_limit: entries.capacity,
        work_generation: 0,
        last_seen_generation: 0,
        waiters: crate::klib::observer::registration::ObserverList::try_new(
            crate::klib::observer::waiter_budget::SOURCE_LIMIT,
            platform,
        )
        .map_err(|_| CqOpenError::AllocationFailed)?,
        _buf: None,
        _storage_charge: charge,
    };
    state.backlog.try_reserve_exact(slots).map_err(|_| CqOpenError::AllocationFailed)?;
    if state.backlog.capacity() > slots {
        return Err(CqOpenError::AllocationFailed);
    }
    if let Some(frame) = frame {
        // Kernel mapping boundary: the caller owns the writable physical frame;
        // alias/admission checks precede all writes to it under the registry.
        state.ring = unsafe { cq::CompletionQueueRing::init_at_phys(frame, cq_entries) }
            .map_err(CqOpenError::Ring)?;
    } else {
        let (buf, ptr) =
            cq::CompletionQueueRing::new_page(cq_entries).map_err(CqOpenError::Ring)?;
        state._buf = Some(buf);
        state.ring = ptr;
    }
    Ok(state)
}

fn open_namespace_with_cq(
    asid: AddressSpaceId,
    capacity: usize,
    cq_entries: u32,
    frame: Option<crate::memory::physical::PAddr>,
) -> Result<(), CqOpenError> {
    let mut entries = empty_as(asid, capacity);
    let platform = crate::memory::budget::platform_identity(asid);
    let mut registry = COMPLETIONS.write();
    check_ring_alias(&registry, asid, DEFAULT_CQ, frame)?;
    let state = stage_cq(asid, &entries, cq_entries, platform, frame)?;
    entries.cqs.insert(DEFAULT_CQ, state);
    if let Some(previous) = registry.insert(asid, entries) {
        for cap in previous.table.keys() {
            assert!(
                crate::capability::remove(asid, *cap, crate::capability::ObjectKind::Completion),
                "replaced completion capability was absent from unified table"
            );
        }
    }
    Ok(())
}

/// Attaches an additional heap-backed completion queue to an address space —
/// one per shard in the per-shard-CQ model (§8.1). Replaces any existing
/// queue with the same id.
pub fn open_cq(asid: AddressSpaceId, cq: CqId, cq_entries: u32) -> Result<(), CqOpenError> {
    attach_cq(asid, cq, cq_entries, None)
}

/// Attaches an additional completion queue whose ring lives on a
/// pre-allocated physical frame (mappable into the user address space).
pub fn open_cq_phys(
    asid: AddressSpaceId,
    cq: CqId,
    ring_frame: crate::memory::physical::PAddr,
    cq_entries: u32,
) -> Result<(), CqOpenError> {
    attach_cq(asid, cq, cq_entries, Some(ring_frame))
}

fn attach_cq(
    asid: AddressSpaceId,
    cq: CqId,
    cq_entries: u32,
    frame: Option<crate::memory::physical::PAddr>,
) -> Result<(), CqOpenError> {
    let platform = crate::memory::budget::platform_identity(asid);
    let mut registry = COMPLETIONS.write();
    check_ring_alias(&registry, asid, cq, frame)?;
    let entries = registry.get_mut(&asid).ok_or(CqOpenError::UnknownAddressSpace)?;
    let state = stage_cq(asid, entries, cq_entries, platform, frame)?;
    replace_cq(entries, cq, state);
    Ok(())
}

/// Kernel-controlled queue replacement discards old undelivered results.
/// Retained detached results must return their submission slots as well as
/// their owning record charges; capability results still live in the table.
fn replace_cq(entries: &mut AsCompletions, cq: CqId, replacement: CqState) {
    if let Some(previous) = entries.cqs.insert(cq, replacement) {
        let discarded = previous
            .backlog
            .iter()
            .filter(|entry| matches!(&entry.owner, BacklogOwner::Detached { .. }))
            .count();
        entries.live = entries
            .live
            .checked_sub(discarded)
            .expect("discarded detached records must count as live");
    }
}

/// Returns a raw pointer to a CQ ring of `asid`, or `None`.
///
/// # Safety
///
/// The caller must ensure the address space cannot be closed or have this CQ
/// replaced for the complete duration of every pointer use. A mutable access
/// additionally requires that no other consumer drains the ring concurrently.
pub unsafe fn cq_ring_of(
    asid: AddressSpaceId,
    cq: CqId,
) -> Option<*mut crate::completion::cq::CompletionQueueRing> {
    let registry = COMPLETIONS.read();
    registry.get(&asid).and_then(|c| c.cqs.get(&cq)).map(|state| state.ring)
}

/// Closes an address space's capability table and frees its CQ ring (if any).
pub fn close_address_space(asid: AddressSpaceId) {
    if let Some(completions) = COMPLETIONS.write().remove(&asid) {
        for cap in completions.table.keys() {
            assert!(
                crate::capability::remove(asid, *cap, crate::capability::ObjectKind::Completion),
                "completion payload capability was absent from unified table"
            );
        }
    }
}

pub(crate) fn completion_of(
    asid: AddressSpaceId,
    cap: CompletionCap,
) -> Result<CompletionRef, CapError> {
    if !crate::capability::contains(asid, cap, crate::capability::ObjectKind::Completion) {
        return Err(CapError::UnknownCap);
    }
    let registry = COMPLETIONS.read();
    let as_completions = registry.get(&asid).ok_or(CapError::UnknownAddressSpace)?;
    let completion = as_completions.table.get(&cap).ok_or(CapError::UnknownCap)?;
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
    submit_captured(asid, buffer, false).map(|(cap, _, _)| cap)
}

fn submit_captured(
    asid: AddressSpaceId,
    buffer: Option<Vec<u8>>,
    event_watch: bool,
) -> Result<(CompletionCap, CompletionRef, Option<watch_budget::Charge>), SubmitError> {
    let platform_identity = crate::memory::budget::platform_identity(asid);
    let mut registry = COMPLETIONS.write();
    let as_completions = registry.get_mut(&asid).ok_or(SubmitError::UnknownAddressSpace)?;
    flush_cq_backlog(as_completions, DEFAULT_CQ);
    if as_completions.live >= as_completions.capacity {
        return Err(SubmitError::WouldBlock);
    }
    let record_charge = reserve_record(asid, as_completions, platform_identity)?;
    let watch_charge = if event_watch {
        let platform = asid == crate::memory::KERNEL_ASID
            || platform_identity.is_some_and(|handle| Some(handle) == as_completions.address_space);
        Some(
            watch_budget::reserve(&as_completions.watch_budget, platform)
                .map_err(|_| SubmitError::WouldBlock)?,
        )
    } else {
        None
    };
    let reservation = crate::capability::reserve_captured(
        asid,
        crate::capability::ObjectKind::Completion,
        as_completions.address_space,
    )
    .map_err(|_| SubmitError::WouldBlock)?;
    let completion = Completion::new(buffer, record_charge)?;
    let cap = reservation.publish().map_err(|_| SubmitError::WouldBlock)?;
    as_completions.table.insert(cap, completion.clone());
    as_completions.live += 1;
    Ok((cap, completion, watch_charge))
}

/// Own an unpublished event-watch submission and its entry reservation.
/// Rollback rechecks object identity, so teardown/reuse cannot revoke a new cap.
pub(crate) struct EventSubmission {
    asid: AddressSpaceId,
    cap: CompletionCap,
    completion: CompletionRef,
    charge: Option<watch_budget::Charge>,
    committed: bool,
}

impl EventSubmission {
    pub(crate) fn new(asid: AddressSpaceId) -> Result<Self, SubmitError> {
        let (cap, completion, charge) = submit_captured(asid, None, true)?;
        Ok(Self {
            asid,
            cap,
            completion,
            charge,
            committed: false,
        })
    }

    pub(crate) fn cap(&self) -> CompletionCap {
        self.cap
    }

    pub(crate) fn completion(&self) -> &CompletionRef {
        &self.completion
    }

    pub(crate) fn take_charge(&mut self) -> watch_budget::Charge {
        self.charge.take().expect("event submission charge transferred twice")
    }

    pub(crate) fn install_watch_observation(
        &self,
        observer: Arc<dyn Observer>,
        registration: crate::klib::observer::registration::Registration<watch_budget::Charge>,
    ) -> Result<bool, SubmitError> {
        self.install_observation(observer, registration, false)
    }

    fn install_worker_observation(
        &self,
        observer: Arc<dyn Observer>,
        registration: crate::klib::observer::registration::Registration<watch_budget::Charge>,
    ) -> Result<(), SubmitError> {
        self.install_observation(observer, registration, true).map(|_| ())
    }

    /// Fence installation against namespace teardown/replacement, including
    /// when a kernel waiter retains the unpublished old completion object.
    fn install_observation(
        &self,
        observer: Arc<dyn Observer>,
        registration: crate::klib::observer::registration::Registration<watch_budget::Charge>,
        worker: bool,
    ) -> Result<bool, SubmitError> {
        let registry = COMPLETIONS.read();
        if !registry.get(&self.asid).is_some_and(|entries| {
            entries.table.get(&self.cap).is_some_and(|live| Arc::ptr_eq(live, &self.completion))
        }) {
            return Err(SubmitError::UnknownAddressSpace);
        }
        if worker {
            self.completion.set_worker_observation(EventObservation {
                _observer: observer,
                _registration: registration,
            });
            Ok(false)
        } else {
            Ok(self.completion.set_event_observation(observer, registration))
        }
    }

    pub(crate) fn commit(mut self) -> CompletionCap {
        self.committed = true;
        self.cap
    }
}

impl Drop for EventSubmission {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut registry = COMPLETIONS.write();
        if let Some(entries) = registry.get_mut(&self.asid)
            && entries.table.get(&self.cap).is_some_and(|live| Arc::ptr_eq(live, &self.completion))
        {
            if let Some(cq) = entries.cqs.get_mut(&DEFAULT_CQ) {
                let operation = self.completion.operation_id();
                cq.backlog.retain(|entry| {
                    !matches!(&entry.owner, BacklogOwner::Capability)
                        || entry.operation != operation
                        || entry.cookie != self.cap
                });
            }
            entries.table.remove(&self.cap);
            entries.live = entries.live.checked_sub(1).expect("staged event slot missing");
            assert!(crate::capability::remove(
                self.asid,
                self.cap,
                crate::capability::ObjectKind::Completion
            ));
        }
        drop(registry);
        self.completion.release_event_observation();
        drop(self.completion.detach_callbacks());
    }
}

/// Roll back a capability-backed submission before it becomes externally
/// visible. This is used by syscall validation that must reserve a bounded
/// completion slot before performing an atomic checked user-memory write.
pub(crate) fn abort_submission(asid: AddressSpaceId, cap: CompletionCap) -> Result<(), CapError> {
    let mut registry = COMPLETIONS.write();
    let as_completions = registry.get_mut(&asid).ok_or(CapError::UnknownAddressSpace)?;
    let completion = as_completions.table.remove(&cap).ok_or(CapError::UnknownCap)?;
    as_completions.live =
        as_completions.live.checked_sub(1).expect("aborted completion must count as live");
    assert!(
        crate::capability::remove(asid, cap, crate::capability::ObjectKind::Completion),
        "aborted completion capability was absent from unified table"
    );
    drop(registry);
    completion.release_event_observation();
    drop(completion.detach_callbacks());
    Ok(())
}

/// Submits a timer operation: creates a capability that auto-completes after
/// `timeout_ms` milliseconds. The returned cap delivers a completion ring entry
/// when the deadline expires, so a user-space service waiting on `cq_wait` is
/// released exactly at the deadline.
pub fn submit_timer(asid: AddressSpaceId, timeout_ms: u64) -> Result<CompletionCap, SubmitError> {
    let platform_identity = crate::memory::budget::platform_identity(asid);
    let (cap, timer_event) = {
        let mut registry = COMPLETIONS.write();
        let entries = registry.get_mut(&asid).ok_or(SubmitError::UnknownAddressSpace)?;
        flush_cq_backlog(entries, DEFAULT_CQ);
        if entries.live >= entries.capacity {
            return Err(SubmitError::WouldBlock);
        }
        let record_charge = reserve_record(asid, entries, platform_identity)?;
        // Tie policy captured before the registry lock to this exact namespace;
        // teardown/reuse must not lend a predecessor's reserved-pool access.
        let platform = asid == crate::memory::KERNEL_ASID
            || (platform_identity.is_some() && platform_identity == entries.address_space);
        let charge = crate::timers::budget::reserve(&entries.timer_budget, platform)
            .map_err(|_| SubmitError::WouldBlock)?;
        let (timer_event, cancel) =
            TimerEvent::charged(ExtDuration::from_millis(timeout_ms as u128), charge)
                .map_err(|_| SubmitError::WouldBlock)?;
        let timer_event =
            crate::timers::PreparedEvent::new(timer_event).map_err(|_| SubmitError::WouldBlock)?;
        let reservation = crate::capability::reserve_captured(
            asid,
            crate::capability::ObjectKind::Completion,
            entries.address_space,
        )
        .map_err(|_| SubmitError::WouldBlock)?;
        let completion = Completion::new(None, record_charge)?;
        let cap = reservation.identity();
        let observer = Arc::new(CompletionTimerObserver {
            asid,
            cap,
            result: OpResult::Ok(0),
            completion: Arc::downgrade(&completion),
        });
        timer_event.event().register_observer(Arc::downgrade(&observer) as Weak<dyn Observer>);
        completion.set_timer_observer(observer, cancel);
        let cap = reservation.publish().map_err(|_| SubmitError::WouldBlock)?;
        entries.table.insert(cap, completion);
        entries.live += 1;
        (cap, timer_event)
    };
    timer_event.enqueue();
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
    let mut submission = EventSubmission::new(asid)?;
    let completion = submission.completion().clone();
    let observer: Arc<dyn Observer> = Arc::try_new(CompletionExitObserver {
        asid,
        cap: submission.cap(),
        result,
        completion: Arc::downgrade(&completion),
    })
    .map_err(|_| SubmitError::WouldBlock)?;
    crate::cpu::scheduler::spawn_worker_after_observe(
        worker_entry,
        Arc::downgrade(&observer),
        submission.take_charge(),
        |registration| submission.install_worker_observation(observer, registration),
    )?;
    Ok(submission.commit())
}

/// Register a completion capability that fires when the EL0 thread `tid`
/// exits. A supplied spawn-time generation prevents a delayed userspace join
/// from attaching to a replacement in the same numeric TID slot.
///
/// Thread IDs are recycled, so a caller that captured a `ServiceDomain`
/// (which records the spawn-time generation) must pass that generation: if
/// the tid slot already holds a different thread, the observed thread is
/// guaranteed gone and the capability completes immediately instead of
/// joining a stranger that may never exit.
pub(crate) fn observe_thread_exit_with_generation(
    asid: AddressSpaceId,
    tid: crate::cpu::scheduler::threads::ThreadId,
    expected_generation: Option<crate::cpu::scheduler::threads::ThreadGeneration>,
) -> Result<CompletionCap, SubmitError> {
    observe_thread_exit_scoped(asid, tid, expected_generation, None)
}

/// EL0 adapter: target authority is restricted to the exact caller domain.
pub(crate) fn observe_own_thread_exit(
    asid: AddressSpaceId,
    tid: crate::cpu::scheduler::threads::ThreadId,
    expected_generation: Option<crate::cpu::scheduler::threads::ThreadGeneration>,
) -> Result<CompletionCap, SubmitError> {
    let handle =
        crate::memory::current_address_space_handle(asid).ok_or(SubmitError::WouldBlock)?;
    let operation = crate::memory::operation::AddressSpaceOperation::acquire(handle)
        .map_err(|_| SubmitError::WouldBlock)?;
    let result = observe_thread_exit_scoped(asid, tid, expected_generation, Some(handle));
    operation.release().map_err(|_| SubmitError::WouldBlock)?;
    result
}

fn observe_thread_exit_scoped(
    asid: AddressSpaceId,
    tid: crate::cpu::scheduler::threads::ThreadId,
    expected_generation: Option<crate::cpu::scheduler::threads::ThreadGeneration>,
    caller: Option<crate::memory::AddressSpaceHandle>,
) -> Result<CompletionCap, SubmitError> {
    let mut submission = EventSubmission::new(asid)?;
    let cap = submission.cap();
    let completion = submission.completion().clone();
    let observer: Arc<dyn Observer> = Arc::try_new(CompletionExitObserver {
        asid,
        cap,
        result: OpResult::Ok(0),
        completion: Arc::downgrade(&completion),
    })
    .map_err(|_| SubmitError::WouldBlock)?;
    let registration = if let Some(caller) = caller {
        crate::cpu::scheduler::observe_thread_exit_in_domain(
            caller,
            tid,
            expected_generation,
            Arc::downgrade(&observer),
            submission.take_charge(),
        )
    } else {
        match expected_generation {
            Some(generation) => crate::cpu::scheduler::observe_thread_exit_with_generation(
                tid,
                generation,
                Arc::downgrade(&observer),
                submission.take_charge(),
            ),
            None => crate::cpu::scheduler::observe_thread_exit(
                tid,
                Arc::downgrade(&observer),
                submission.take_charge(),
            ),
        }
    };
    match registration {
        Ok(registration) => {
            if submission.install_watch_observation(observer, registration)? {
                let _ = complete_registered(asid, cap, completion, OpResult::Cancelled);
            }
        }
        Err(crate::cpu::scheduler::system_scheduler::Error::InvalidThread) => {
            // The thread is gone; its exit already happened. Complete now so
            // the joiner observes a terminal state immediately.
            let _ = complete_registered(asid, cap, completion, OpResult::Ok(0));
        }
        Err(_) => return Err(SubmitError::WouldBlock),
    }
    Ok(submission.commit())
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
    complete_registered(asid, cap, completion, result)
}

/// Complete only the exact object captured by an asynchronous producer.
/// Unlike resolving a numeric handle again, this cannot affect a replacement
/// address-space namespace. Callers retain/upgrade the originally observed Arc.
pub(crate) fn complete_registered(
    asid: AddressSpaceId,
    cap: CompletionCap,
    completion: CompletionRef,
    result: OpResult,
) -> Result<(), CapError> {
    // Transition and publish under one registry hold so a concurrent poll or
    // close cannot remove the capability between the terminal transition and
    // the CQ insertion and leave a terminal operation untracked. The ring
    // write must also happen-before the wake, so a userspace consumer that
    // blocks in `wait` and drains the ring the moment it is woken observes
    // the entry. Capability-backed completions go to the default queue.
    {
        let mut registry = COMPLETIONS.write();
        // A callback captured before teardown must never complete a new
        // namespace occupant that happens to reuse its ASID/cap values.
        if !registry
            .get(&asid)
            .and_then(|entries| entries.table.get(&cap))
            .is_some_and(|registered| Arc::ptr_eq(registered, &completion))
        {
            return Err(CapError::UnknownCap);
        }
        let Some(effective) = completion.complete(result) else {
            // Already terminal: idempotent no-op, no duplicate CQ entry.
            return Ok(());
        };
        if let Some(as_completions) = registry.get_mut(&asid)
            && as_completions
                .table
                .get(&cap)
                .is_some_and(|registered| Arc::ptr_eq(registered, &completion))
            && let Some(cq_state) = as_completions.cqs.get_mut(&DEFAULT_CQ)
        {
            let op = completion.operation_id();
            let flushed_detached =
                post_to_cq(cq_state, op, cap, &effective, BacklogOwner::Capability)
                    .flushed
                    .detached;
            as_completions.live = as_completions.live.saturating_sub(flushed_detached);
            cq_state.work_generation = cq_state.work_generation.wrapping_add(1);
            crate::debug_trace::trace(
                crate::debug_trace::TAG_COMPLETE,
                asid as u64,
                cq_state.work_generation,
                cap,
            );
        }
    }

    signal_cq(asid, DEFAULT_CQ);
    completion.signal();

    Ok(())
}

/// Posts one entry to a queue's ring, spilling to its non-lossy backlog when
/// the ring is full. Any backlog is flushed first so ordering is preserved.
struct PostOutcome {
    flushed: FlushOutcome,
    delivered_current: bool,
}

fn post_to_cq(
    cq_state: &mut CqState,
    operation: u64,
    cookie: u64,
    result: &OpResult,
    owner: BacklogOwner,
) -> PostOutcome {
    let (status, val) = crate::completion::cq::op_result_to_fields(result);
    let flushed = flush_backlog(cq_state);
    let delivered_current =
        cq_state.backlog.is_empty() && cq_state.push_completion(operation, cookie, status, val);
    if !delivered_current {
        assert!(
            cq_state.backlog.len() < cq_state.retained_limit,
            "completion backlog exceeded address-space submission capacity"
        );
        cq_state.backlog.push_back(BacklogEntry {
            operation,
            cookie,
            status,
            result: val,
            owner,
        });
    }
    PostOutcome {
        flushed,
        delivered_current,
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
    let platform_identity = crate::memory::budget::platform_identity(asid);
    let mut registry = COMPLETIONS.write();
    let as_completions = registry.get_mut(&asid).ok_or(SubmitError::UnknownAddressSpace)?;
    flush_cq_backlog(as_completions, cq);
    if !as_completions.cqs.contains_key(&cq) {
        return Err(SubmitError::NoCompletionQueue);
    }
    if as_completions.live >= as_completions.capacity {
        return Err(SubmitError::WouldBlock);
    }
    let record_charge = reserve_record(asid, as_completions, platform_identity)?;
    let operation = alloc_operation_id();
    as_completions.detached.insert(
        operation,
        DetachedOp {
            user_data,
            cq,
            cancel_pending: false,
            _timer_observer: None,
            timer_cancel: None,
            record_charge,
        },
    );
    as_completions.live += 1;
    Ok(operation)
}

/// Submit a timer delivered exclusively through a completion queue.
pub fn submit_detached_timer(
    asid: AddressSpaceId,
    cq: CqId,
    timeout_ms: u64,
    user_data: u64,
) -> Result<OperationId, SubmitError> {
    let platform_identity = crate::memory::budget::platform_identity(asid);
    let (operation, timer_event) = {
        let mut registry = COMPLETIONS.write();
        let entries = registry.get_mut(&asid).ok_or(SubmitError::UnknownAddressSpace)?;
        flush_cq_backlog(entries, cq);
        if !entries.cqs.contains_key(&cq) {
            return Err(SubmitError::NoCompletionQueue);
        }
        if entries.live >= entries.capacity {
            return Err(SubmitError::WouldBlock);
        }
        let record_charge = reserve_record(asid, entries, platform_identity)?;
        let platform = asid == crate::memory::KERNEL_ASID
            || (platform_identity.is_some() && platform_identity == entries.address_space);
        let charge = crate::timers::budget::reserve(&entries.timer_budget, platform)
            .map_err(|_| SubmitError::WouldBlock)?;
        let (timer_event, cancel) =
            TimerEvent::charged(ExtDuration::from_millis(timeout_ms as u128), charge)
                .map_err(|_| SubmitError::WouldBlock)?;
        let timer_event =
            crate::timers::PreparedEvent::new(timer_event).map_err(|_| SubmitError::WouldBlock)?;
        let operation = alloc_operation_id();
        let observer = Arc::new(DetachedTimerObserver {
            asid,
            operation,
        });
        timer_event.event().register_observer(Arc::downgrade(&observer) as Weak<dyn Observer>);
        entries.detached.insert(
            operation,
            DetachedOp {
                user_data,
                cq,
                cancel_pending: false,
                _timer_observer: Some(observer),
                timer_cancel: Some(TimerCancellation(Some(cancel))),
                record_charge,
            },
        );
        entries.live += 1;
        (operation, timer_event)
    };
    timer_event.enqueue();
    Ok(operation)
}

/// Completes a capability-free operation: posts `(user_data, result)` to the
/// operation's queue (or its non-lossy backlog), reclaims the addressable
/// operation record, and wakes that queue's waiters. A backlogged record keeps
/// its submission slot until it reaches the ring, bounding retained records by
/// the address-space capacity. The effective result is forced to
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
            let outcome = post_to_cq(
                cq_state,
                operation,
                detached.user_data,
                &effective,
                BacklogOwner::Detached {
                    _charge: detached.record_charge,
                },
            );
            cq_state.work_generation = cq_state.work_generation.wrapping_add(1);
            crate::debug_trace::trace(
                crate::debug_trace::TAG_COMPLETE_DETACHED,
                asid as u64,
                cq_state.work_generation,
                operation,
            );
            let delivered = outcome.flushed.detached + usize::from(outcome.delivered_current);
            as_completions.live = as_completions.live.saturating_sub(delivered);
        } else {
            // The queue disappeared after submission, so no delivery can be
            // retained. Release this operation's submission slot.
            as_completions.live = as_completions.live.saturating_sub(1);
        }
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
    let timer = detached.timer_cancel.take();
    drop(registry);
    if timer.is_some() {
        drop(timer);
        // Only timer work is synchronously stoppable. Other operations retain
        // their buffers/submission slots until their real producer completes.
        let _ = complete_detached(asid, operation, OpResult::Cancelled);
    }
    Ok(CancelState::CancelRequested)
}

#[derive(Clone, Copy, Default)]
struct FlushOutcome {
    total: usize,
    detached: usize,
}

/// Writes as many retained entries as fit, preserving order. Reports how many
/// entries became visible and how many detached entries therefore released
/// submission slots.
fn flush_backlog(cq_state: &mut CqState) -> FlushOutcome {
    if cq_state.backlog.is_empty() {
        return FlushOutcome::default();
    }

    let mut outcome = FlushOutcome::default();
    while cq_state.ring_has_space() {
        let Some(entry) = cq_state.backlog.pop_front() else {
            break;
        };
        let written =
            cq_state.push_completion(entry.operation, entry.cookie, entry.status, entry.result);
        if !written {
            // A racing shared-tail update made the ring full; retain the
            // entry and retry after the consumer advances.
            cq_state.backlog.push_front(entry);
            break;
        }
        outcome.total += 1;
        if matches!(&entry.owner, BacklogOwner::Detached { .. }) {
            outcome.detached += 1;
        }
    }
    if outcome.total != 0 {
        // Retained records becoming visible are new work even if the original
        // completion generation was consumed before ring space was available.
        cq_state.work_generation = cq_state.work_generation.wrapping_add(1);
    }
    outcome
}

fn flush_cq_backlog(as_completions: &mut AsCompletions, cq: CqId) -> FlushOutcome {
    let outcome = as_completions.cqs.get_mut(&cq).map(flush_backlog).unwrap_or_default();
    as_completions.live = as_completions.live.saturating_sub(outcome.detached);
    outcome
}

fn signal_cq(asid: AddressSpaceId, cq: CqId) {
    let (count, waiters) = {
        let registry = COMPLETIONS.read();
        let Some(cq_state) = registry.get(&asid).and_then(|c| c.cqs.get(&cq)) else {
            return;
        };
        (cq_state.waiters.registered(), cq_state.waiters.drain())
    };
    waiters.notify();
    crate::debug_trace::trace(
        crate::debug_trace::TAG_SIGNAL_CQ,
        asid as u64,
        cq as u64,
        count as u64,
    );
}

struct CqObservable {
    asid: AddressSpaceId,
    cq: CqId,
}

impl Observable for CqObservable {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &crate::klib::observer::WaitSponsor,
    ) -> Result<
        crate::klib::observer::WaitRegistration,
        crate::klib::observer::registration::RegistrationError,
    > {
        let registry = COMPLETIONS.read();
        let state = registry
            .get(&self.asid)
            .and_then(|entries| entries.cqs.get(&self.cq))
            .ok_or(crate::klib::observer::registration::RegistrationError::Closed)?;
        if state.work_generation != state.last_seen_generation {
            return Ok(crate::klib::observer::WaitRegistration::ready());
        }
        sponsor.register(&state.waiters, observer)
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

    while !completion.is_terminal() {
        let registration = SYSTEM_SCHEDULER.read().block_thread_with_constraint_generation(
            tid,
            completion.as_ref() as &dyn Observable,
            crate::cpu::scheduler::threads::MigrationConstraint::GeneralWait,
        );
        match registration {
            Ok(generation) => {
                // Close the lost-wake window after successful registration.
                if completion.is_terminal() {
                    let _ = SYSTEM_SCHEDULER.read().submit_woken_thread(tid, generation);
                }
            }
            Err(crate::cpu::scheduler::system_scheduler::Error::WaitRegistrationFailed) => {
                // The void, untimed ABI must preserve terminal-wait semantics:
                // a ReadOperation may release its borrowed buffer on return.
                // Stay runnable and retry cooperatively under admission pressure.
            }
            Err(_) => return Err(CapError::UnknownCap),
        }
        yield_lp();
        // A competing wake is not proof that the producer has terminated.
    }
    Ok(())
}

/// Blocks the calling thread until `cap` reaches a terminal completion or
/// `timeout_ms` elapses. Returns `true` if terminal, `false` on timeout or
/// failed waiter admission. The capability remains live in the latter cases.
/// Event-driven like [`wait`], with a timer watchdog so a caller waiting on
/// an event that never fires fails loudly instead of hanging silently.
pub fn wait_timeout(
    asid: AddressSpaceId,
    cap: CompletionCap,
    timeout_ms: u64,
) -> Result<bool, CapError> {
    let completion = completion_of(asid, cap)?;
    if completion.is_terminal() {
        return Ok(true);
    }
    Ok(crate::cpu::scheduler::block_until(
        completion.as_ref() as &dyn Observable,
        timeout_ms,
        || completion.is_terminal(),
    ))
}

/// Requests cancellation of an in-flight operation. See [`CancelState`]. Any
/// transferred buffer is retained until the terminal completion hands it back.
pub fn cancel(asid: AddressSpaceId, cap: CompletionCap) -> Result<CancelState, CapError> {
    let completion = completion_of(asid, cap)?;
    let state = completion.cancel();
    if state == CancelState::CancelRequested {
        let (timer, event) = {
            let mut inner = completion.inner.lock();
            (inner.timer_cancel.take(), inner.event_observation.take())
        };
        if timer.is_some() || event.is_some() {
            drop((timer, event));
            let _ = complete_registered(asid, cap, completion, OpResult::Cancelled);
        }
    }
    Ok(state)
}

pub(crate) fn timer_events_used(asid: AddressSpaceId) -> usize {
    COMPLETIONS.read().get(&asid).map_or(0, |entries| entries.timer_budget.used())
}

pub(crate) fn waiter_count(asid: AddressSpaceId, cap: CompletionCap) -> usize {
    completion_of(asid, cap).map_or(0, |completion| completion.waiters.registered())
}

pub(crate) fn cq_waiter_count(asid: AddressSpaceId, cq: CqId) -> usize {
    COMPLETIONS
        .read()
        .get(&asid)
        .and_then(|entries| entries.cqs.get(&cq))
        .map_or(0, |state| state.waiters.registered())
}

pub(crate) fn record_admission(asid: AddressSpaceId) -> Option<Arc<budget::DomainBudget>> {
    COMPLETIONS.read().get(&asid).map(|entries| entries.record_budget.clone())
}

pub(crate) fn cq_admission(asid: AddressSpaceId) -> Option<Arc<cq_budget::DomainBudget>> {
    COMPLETIONS.read().get(&asid).map(|entries| entries.cq_budget.clone())
}

pub(crate) fn watch_admission(asid: AddressSpaceId) -> Option<Arc<watch_budget::DomainBudget>> {
    COMPLETIONS.read().get(&asid).map(|entries| entries.watch_budget.clone())
}

/// Revokes a completed or already-drained capability. Fails with
/// [`CapError::NotComplete`] if the operation is still in flight (neither
/// completed nor drained).
pub fn close(asid: AddressSpaceId, cap: CompletionCap) -> Result<(), CapError> {
    let completion = completion_of(asid, cap)?;
    close_registered(asid, cap, completion)
}

/// Close only the captured object, not a replacement that reused its handle.
pub(crate) fn close_registered(
    asid: AddressSpaceId,
    cap: CompletionCap,
    completion: CompletionRef,
) -> Result<(), CapError> {
    if !completion.is_reclaimable() {
        return Err(CapError::NotComplete);
    }
    let operation = completion.operation_id();
    let mut registry = COMPLETIONS.write();
    let as_completions = registry.get_mut(&asid).ok_or(CapError::UnknownAddressSpace)?;
    if !as_completions
        .table
        .get(&cap)
        .is_some_and(|registered| Arc::ptr_eq(registered, &completion))
    {
        return Err(CapError::UnknownCap);
    }
    if let Some(cq_state) = as_completions.cqs.get_mut(&DEFAULT_CQ) {
        cq_state.backlog.retain(|entry| {
            !matches!(&entry.owner, BacklogOwner::Capability)
                || entry.operation != operation
                || entry.cookie != cap
        });
    }
    as_completions.table.remove(&cap).ok_or(CapError::UnknownCap)?;
    let revoked = crate::capability::remove(asid, cap, crate::capability::ObjectKind::Completion);
    assert!(revoked, "completion payload capability was absent from unified table");
    as_completions.live = as_completions.live.saturating_sub(1);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveError {
    UnknownAddressSpace,
    UnknownCap,
    ResourceLimit,
}

/// Kernel callback owner, independent of the watched operation's lifetime.
/// Dropping it cancels only the subscription, never the operation. A callback
/// already captured for notification may race cancellation. No syscall exposes
/// callback objects; application completions continue to use the owned runtime.
#[must_use = "retain the callback owner until notification or cancellation"]
pub struct CompletionObservation {
    _registration: Option<crate::klib::observer::registration::Registration<watch_budget::Charge>>,
    _observer: Arc<dyn Observer>,
}

impl core::fmt::Debug for CompletionObservation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CompletionObservation")
            .field("has_token", &self._registration.is_some())
            .finish_non_exhaustive()
    }
}

/// Register a kernel callback against this namespace's exact completion.
/// Registration is fallible and cancellation is owned. An already-terminal
/// completion invokes the callback immediately, outside every registry/source
/// guard; callback invocation is not a scheduler parking primitive.
pub fn observe(
    asid: AddressSpaceId,
    cap: CompletionCap,
    observer: Arc<dyn Observer>,
) -> Result<CompletionObservation, ObserveError> {
    let completion = {
        let registry = COMPLETIONS.read();
        let entries = registry.get(&asid).ok_or(ObserveError::UnknownAddressSpace)?;
        entries.table.get(&cap).ok_or(ObserveError::UnknownCap)?.clone()
    };
    observe_registered(asid, cap, &completion, observer)
}

/// Asynchronous kernel callers carrying a previously captured operation must
/// use this exact-object variant rather than resolving reusable numeric IDs.
pub(crate) fn observe_registered(
    asid: AddressSpaceId,
    cap: CompletionCap,
    captured: &CompletionRef,
    observer: Arc<dyn Observer>,
) -> Result<CompletionObservation, ObserveError> {
    let platform_identity = crate::memory::budget::platform_identity(asid);
    let registration = {
        let registry = COMPLETIONS.read();
        let entries = registry.get(&asid).ok_or(ObserveError::UnknownAddressSpace)?;
        if entries.address_space.is_some_and(|handle| !crate::memory::budget::accepting(handle)) {
            return Err(ObserveError::UnknownAddressSpace);
        }
        let completion = entries.table.get(&cap).ok_or(ObserveError::UnknownCap)?;
        if !Arc::ptr_eq(completion, captured) {
            return Err(ObserveError::UnknownCap);
        }
        // Serialize the terminal check with the operation transition. Insertion
        // either precedes terminal publication or becomes an immediate callback;
        // it cannot append to an already-drained source and lose notification.
        let mut inner = completion.inner.lock();
        if matches!(inner.state, OpState::Completed(_) | OpState::Observed) {
            None
        } else {
            let platform = asid == crate::memory::KERNEL_ASID
                || platform_identity.is_some_and(|handle| Some(handle) == entries.address_space);
            let charge = watch_budget::reserve(&entries.watch_budget, platform)
                .map_err(|_| ObserveError::ResourceLimit)?;
            if inner.callbacks.is_none() {
                inner.callbacks = Some(
                    crate::klib::observer::registration::ObserverList::try_new(
                        watch_budget::MAX_COMPLETION_CALLBACKS,
                        charge.platform(),
                    )
                    .map_err(|_| ObserveError::ResourceLimit)?,
                );
            }
            Some(
                inner
                    .callbacks
                    .as_ref()
                    .unwrap()
                    .register(Arc::downgrade(&observer), charge)
                    .map_err(|_| ObserveError::ResourceLimit)?,
            )
        }
    };
    let ready = registration.is_none();
    let observation = CompletionObservation {
        _registration: registration,
        _observer: observer,
    };
    if ready {
        observation._observer.clone().notify();
    }
    Ok(observation)
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
    let Some(as_completions) = registry.get_mut(&asid) else {
        return 0;
    };
    flush_cq_backlog(as_completions, cq);
    as_completions.cqs.get(&cq).map(CqState::ring_pending).unwrap_or(0)
}

/// Inspection: the monotonic work generation of queue `cq` of `asid`.
///
/// Bumped by every explicit [`wake`] and completion post. The device
/// self-test uses it to prove a deferred wake queued for a retired interrupt
/// route never reaches a replacement that reuses the same ASID/queue tuple,
/// independently of unrelated wakes drained from the global queue.
pub fn cq_work_generation(asid: AddressSpaceId, cq: CqId) -> u64 {
    let registry = COMPLETIONS.read();
    registry
        .get(&asid)
        .and_then(|as_completions| as_completions.cqs.get(&cq))
        .map(|state| state.work_generation)
        .unwrap_or(0)
}

/// Posts an explicit wake to the waiters of one queue (architecture doc
/// §7.3/§9.4): bumps the work generation so a thread blocked in
/// [`wait_on_cq`]/[`wait_on_cq_timeout`] returns even though no completion
/// entry was posted.  Used by userspace reactors (peer shard interrupts a
/// blocking CQ wait) and by the IPC layer (endpoint-bound CQ wake).
pub fn wake(asid: AddressSpaceId, cq: CqId) {
    if let Some(notification) = prepare_wake(asid, cq) {
        notification.notify();
    }
}

/// Publish work and detach the exact queue's current waiters under one registry
/// guard, without invoking them. Callers fencing another lifecycle may prepare
/// under its guard, but must release every subsystem guard before notifying.
/// No numeric destination is resolved again after detachment.
pub(crate) fn prepare_wake(
    asid: AddressSpaceId,
    cq: CqId,
) -> Option<
    crate::klib::observer::registration::NotificationBatch<
        crate::klib::observer::waiter_budget::Charge,
    >,
> {
    let mut registry = COMPLETIONS.write();
    let state = registry.get_mut(&asid)?.cqs.get_mut(&cq)?;
    state.work_generation = state.work_generation.wrapping_add(1);
    crate::debug_trace::trace(
        crate::debug_trace::TAG_WAKE,
        asid as u64,
        state.work_generation,
        cq as u64,
    );
    let count = state.waiters.registered();
    let notification = state.waiters.drain();
    crate::debug_trace::trace(
        crate::debug_trace::TAG_SIGNAL_CQ,
        asid as u64,
        cq as u64,
        count as u64,
    );
    Some(notification)
}

/// Kernel fixture: a detached wake batch belongs to the captured CQ, not a
/// later namespace occupying the same numeric ASID/CQ. Reentrant callbacks
/// also exercise the rule that no registry/device guard survives notification.
pub(crate) fn test_prepared_wake_identity() {
    use core::sync::atomic::{
        AtomicUsize,
        Ordering,
    };

    use crate::klib::observer::{
        CallOnNotify,
        WaitSponsor,
    };

    const CLIENT: usize = 0x5e10;
    let sponsor = WaitSponsor::new(false);
    let old_calls = Arc::new(AtomicUsize::new(0));
    let old_counter = old_calls.clone();
    let old: Arc<dyn Observer> = CallOnNotify::new(move || {
        old_counter.fetch_add(1, Ordering::SeqCst);
        assert_eq!(cq_work_generation(CLIENT, 0), 0);
        assert_eq!(
            crate::device::grant_interrupt(0, 42),
            Err(crate::device::DeviceError::InvalidAddressSpace)
        );
    });
    open_address_space_with_cq(CLIENT, 8, 8).unwrap();
    let old_registration = CqObservable {
        asid: CLIENT,
        cq: 0,
    }
    .try_register_waiter(Arc::downgrade(&old), &sponsor)
    .unwrap();
    let batch = prepare_wake(CLIENT, 0).unwrap();
    assert_eq!(cq_work_generation(CLIENT, 0), 1);
    assert_eq!(sponsor.used(), 1);
    close_address_space(CLIENT);
    open_address_space_with_cq(CLIENT, 8, 8).unwrap();
    let new_calls = Arc::new(AtomicUsize::new(0));
    let new_counter = new_calls.clone();
    let new: Arc<dyn Observer> = CallOnNotify::new(move || {
        new_counter.fetch_add(1, Ordering::SeqCst);
    });
    let new_registration = CqObservable {
        asid: CLIENT,
        cq: 0,
    }
    .try_register_waiter(Arc::downgrade(&new), &sponsor)
    .unwrap();
    batch.notify();
    assert_eq!(old_calls.load(Ordering::SeqCst), 1);
    assert_eq!(new_calls.load(Ordering::SeqCst), 0);
    assert_eq!(cq_work_generation(CLIENT, 0), 0);
    wake(CLIENT, 0);
    assert_eq!(new_calls.load(Ordering::SeqCst), 1);
    assert_eq!(sponsor.used(), 0);
    drop((old_registration, new_registration));
    close_address_space(CLIENT);
    assert!(prepare_wake(CLIENT, 0).is_none());
    crate::logln!("[device] prepared CQ wake fenced against exact numeric namespace reuse");
}

/// Exercise actual deferred IRQ dispatch with a callback that reenters both
/// registries. This fixture owns its registration and waits for an LP that may
/// already have claimed the global mailbox, rather than assuming local drain
/// is always the winning consumer.
pub(crate) fn test_irq_wake_reentrancy(asid: AddressSpaceId, intid: u32) {
    use core::sync::atomic::{
        AtomicUsize,
        Ordering,
    };

    use crate::klib::observer::{
        CallOnNotify,
        WaitSponsor,
    };

    let sponsor = WaitSponsor::new(false);
    let baseline = cq_work_generation(asid, 0);
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let callback: Arc<dyn Observer> = CallOnNotify::new(move || {
        assert!(cq_work_generation(asid, 0) > baseline);
        assert_eq!(
            crate::device::grant_interrupt(0, intid),
            Err(crate::device::DeviceError::InvalidAddressSpace)
        );
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let registration = {
        let registry = COMPLETIONS.read();
        let state = registry.get(&asid).unwrap().cqs.get(&0).unwrap();
        // Fixture an already waiting reader independently of the queue's
        // last_seen cursor; this pseudo domain has no scheduled reactor.
        sponsor.register(&state.waiters, Arc::downgrade(&callback)).unwrap()
    };
    assert!(crate::device::deliver_interrupt(intid));
    let deadline = crate::self_test::results::Deadline::after_millis(5_000);
    while calls.load(Ordering::SeqCst) == 0 {
        crate::device::drain_deferred_wakes();
        deadline.assert_pending("reentrant deferred IRQ notification");
        core::hint::spin_loop();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(sponsor.used(), 0);
    drop(registration);
    crate::logln!(
        "[device] deferred IRQ notification reentered device/CQ registries outside guards"
    );
}

/// Blocks the calling thread until queue `cq` of `asid` receives new work
/// (a completion, an explicit wake, or an endpoint-bound message).  Uses a
/// per-CQ monotonic work-generation counter rather than checking the ring's
/// `pending()` count, so callers that poll individual completions via
/// `poll(cap)` and never drain the shared ring are not stuck in a busy-spin.
pub fn wait_on_cq(asid: AddressSpaceId, cq: CqId, _min_complete: u32) {
    let Some(tid) = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid() else {
        return;
    };
    let observable = CqObservable {
        asid,
        cq,
    };

    // Fast path: work arrived before the first cq_wait call.
    {
        let mut registry = COMPLETIONS.write();
        if let Some(as_completions) = registry.get_mut(&asid) {
            flush_cq_backlog(as_completions, cq);
            if let Some(cq_state) = as_completions.cqs.get_mut(&cq)
                && charlotte_lifecycle::classify_timed_wait(
                    cq_state.last_seen_generation,
                    cq_state.work_generation,
                ) == charlotte_lifecycle::TimedWaitOutcome::Work
            {
                crate::debug_trace::trace(
                    crate::debug_trace::TAG_CQ_WAIT_FAST,
                    asid as u64,
                    cq_state.work_generation,
                    cq_state.last_seen_generation,
                );
                cq_state.last_seen_generation = cq_state.work_generation;
                return;
            }
        }
    }

    loop {
        // Work may have arrived between the fast-path check and this loop.
        // Consume that generation atomically; returning without advancing
        // last_seen would make the following wait report the same work again.
        {
            let mut registry = COMPLETIONS.write();
            if let Some(cq_state) = registry.get_mut(&asid).and_then(|c| c.cqs.get_mut(&cq))
                && cq_state.work_generation != cq_state.last_seen_generation
            {
                cq_state.last_seen_generation = cq_state.work_generation;
                return;
            }
        }

        crate::debug_trace::trace(
            crate::debug_trace::TAG_CQ_WAIT_ENTER,
            asid as u64,
            {
                let registry = COMPLETIONS.read();
                registry
                    .get(&asid)
                    .and_then(|c| c.cqs.get(&cq))
                    .map(|s| s.work_generation)
                    .unwrap_or(0)
            },
            {
                let registry = COMPLETIONS.read();
                registry
                    .get(&asid)
                    .and_then(|c| c.cqs.get(&cq))
                    .map(|s| s.last_seen_generation)
                    .unwrap_or(0)
            },
        );

        let generation = match SYSTEM_SCHEDULER.read().block_thread_with_constraint_generation(
            tid,
            &observable,
            crate::cpu::scheduler::threads::MigrationConstraint::CompletionQueueWait,
        ) {
            Ok(generation) => generation,
            Err(_) => return,
        };

        // Lost-wake guard: if work arrived while the waker was being
        // registered, re-admit the thread before it yields.
        {
            let registry = COMPLETIONS.read();
            if let Some(cq_state) = registry.get(&asid).and_then(|c| c.cqs.get(&cq))
                && cq_state.work_generation != cq_state.last_seen_generation
            {
                crate::debug_trace::trace(
                    crate::debug_trace::TAG_CQ_WAIT_GUARD,
                    asid as u64,
                    cq_state.work_generation,
                    cq_state.last_seen_generation,
                );
                let _ = SYSTEM_SCHEDULER.read().submit_woken_thread(tid, generation);
            }
        }

        yield_lp();

        // On resume, update last_seen and return if new work arrived.
        {
            let mut registry = COMPLETIONS.write();
            if let Some(cq_state) = registry.get_mut(&asid).and_then(|c| c.cqs.get_mut(&cq)) {
                // If the generation hasn't changed, we were woken
                // spuriously — loop and block again.
                if cq_state.work_generation == cq_state.last_seen_generation {
                    continue;
                }
                crate::debug_trace::trace(
                    crate::debug_trace::TAG_CQ_WAIT_RESUME,
                    asid as u64,
                    cq_state.work_generation,
                    cq_state.last_seen_generation,
                );
                cq_state.last_seen_generation = cq_state.work_generation;
                return;
            }
        }
        // CQ was removed while we slept — nothing to wait on.
        return;
    }
}

/// Like [`wait_on_cq`] but also returns when `timeout_ms` elapses. Returns
/// whether the work-generation condition was met (`true`) or the deadline
/// fired first or waiter admission failed (`false`).
pub fn wait_on_cq_timeout(
    asid: AddressSpaceId,
    cq: CqId,
    _min_complete: u32,
    timeout_ms: u64,
) -> bool {
    use crate::klib::time::duration::ExtDuration;

    struct CqTimeoutWake {
        tid: crate::cpu::scheduler::threads::ThreadId,
        generation: crate::cpu::scheduler::threads::ThreadGeneration,
    }
    impl Observer for CqTimeoutWake {
        fn notify(self: Arc<Self>) {
            let _ = SYSTEM_SCHEDULER.read().submit_woken_thread(self.tid, self.generation);
        }
    }

    let Some(tid) = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid() else {
        return false;
    };

    {
        let mut registry = COMPLETIONS.write();
        if let Some(as_completions) = registry.get_mut(&asid) {
            flush_cq_backlog(as_completions, cq);
            if let Some(cq_state) = as_completions.cqs.get_mut(&cq)
                && charlotte_lifecycle::classify_timed_wait(
                    cq_state.last_seen_generation,
                    cq_state.work_generation,
                ) == charlotte_lifecycle::TimedWaitOutcome::Work
            {
                cq_state.last_seen_generation = cq_state.work_generation;
                return true;
            }
        }
    }

    let observable = CqObservable {
        asid,
        cq,
    };
    // Publishing Blocked and installing its watchdog must be non-preemptible.
    let setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
    let Some((expected_generation, sponsor)) = crate::timers::thread_timer_context(tid) else {
        return false;
    };
    let Ok(timeout_obs) = Arc::try_new(CqTimeoutWake {
        tid,
        generation: expected_generation,
    }) else {
        return false;
    };
    let Ok((timer_event, timeout_handle)) =
        crate::timers::prepare_watchdog(ExtDuration::from_millis(timeout_ms as u128), &sponsor)
    else {
        return false;
    };
    timer_event.event().register_observer(Arc::downgrade(&timeout_obs) as Weak<dyn Observer>);
    let generation = match SYSTEM_SCHEDULER.read().block_thread_with_constraint_generation(
        tid,
        &observable,
        crate::cpu::scheduler::threads::MigrationConstraint::CompletionQueueWait,
    ) {
        Ok(generation) => generation,
        Err(_) => return false,
    };

    debug_assert_eq!(generation, expected_generation);
    timer_event.enqueue();

    {
        let registry = COMPLETIONS.read();
        if let Some(cq_state) = registry.get(&asid).and_then(|c| c.cqs.get(&cq))
            && cq_state.work_generation != cq_state.last_seen_generation
        {
            let _ = SYSTEM_SCHEDULER.read().submit_woken_thread(tid, generation);
        }
    }

    drop(setup);
    yield_lp();

    // A CQ notification may win before the deadline. Remove its now-useless
    // watchdog rather than leaving an anonymous event to churn through the
    // timer queue later. Cancellation is harmless if the timer already fired.
    let _ = crate::timers::cancel_event(timeout_handle);

    // Ready admission has already dropped the owning CQ registration, even
    // when the watchdog wins or another strong Waker reference is retained.

    let mut registry = COMPLETIONS.write();
    if let Some(cq_state) = registry.get_mut(&asid).and_then(|c| c.cqs.get_mut(&cq))
        && charlotte_lifecycle::classify_timed_wait(
            cq_state.last_seen_generation,
            cq_state.work_generation,
        ) == charlotte_lifecycle::TimedWaitOutcome::Work
    {
        cq_state.last_seen_generation = cq_state.work_generation;
        return true;
    }
    false
}
