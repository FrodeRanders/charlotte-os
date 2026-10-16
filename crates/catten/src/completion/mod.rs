//! # Completion-Capability Subsystem (Option C prototype)
//!
//! This module is the first in-kernel prototype of the async syscall /
//! completion-capability ABI specified in `docs/architecture/async-syscall-abi.md`. It builds
//! the kernel side of the boundary directly on facilities that already exist:
//!
//! - bounded per-address-space admitted maps retain capability and detached-operation metadata; a
//!   [`CompletionCap`] names an in-flight or completed kernel object through unified authority;
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
mod callbacks;
mod capability_metadata;
mod capability_preparation;
pub(crate) use callbacks::observe_registered;
pub use callbacks::{
    CompletionObservation,
    ObserveError,
    observe,
};
use capability_metadata::{
    CapabilityRecord,
    RetiredCapability,
};
pub mod cq;
pub(crate) mod cq_budget;
mod detached_preparation;
mod detached_tests;
pub(crate) mod exit_tests;
mod namespace_preparation;
use namespace_preparation::RetiredNamespace;
mod operation_id_tests;
mod operation_ids;
mod queue_preparation;
use queue_preparation::RetiredCq;
mod terminal_tests;
pub(crate) mod timer_tests;
mod waiter_tests;
pub(crate) mod watch_budget;

use alloc::{
    collections::VecDeque,
    sync::{
        Arc,
        Weak,
    },
    vec::Vec,
};
use core::{
    mem::ManuallyDrop,
    ops::{
        Deref,
        DerefMut,
    },
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
        collections::retirement_list::{
            AdmittedMap,
            PreparedEntry,
            RetiredEntry,
        },
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
/// Identities in `1..u64::MAX` are never reused within a boot; `u64::MAX` stays
/// reserved for syscall submission failure. Exhaustion permanently
/// rejects new submissions, including after namespace teardown/replacement.
pub type OperationId = u64;

static OPERATION_IDS: operation_ids::OperationIds = operation_ids::OperationIds::new(0);

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
    /// Submission/resource admission failed, including permanent operation-ID
    /// exhaustion. This is not evidence that retrying can make progress.
    WouldBlock,
    /// No capability table is open for this address space.
    UnknownAddressSpace,
    /// A capability-free ([`submit_detached`]) submission needs a completion
    /// queue to deliver its result, but its captured queue is absent or replaced.
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
    /// Source, waiter or watchdog admission rejected without completing the operation.
    WaitAdmissionFailed,
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

/// Installation returns every replaced or rejected owner to the caller. Its
/// explicit disposal must follow both inner and containing registry unlock.
#[must_use]
pub(crate) struct ObservationInstallation {
    cancellation_won: bool,
    unused: Option<EventObservation>,
}

impl ObservationInstallation {
    pub(crate) fn finish(self) -> bool {
        drop(self.unused);
        self.cancellation_won
    }
}

/// One terminal transition retains its detached producer subscriptions until
/// CQ publication and registry unlock. This is not a physical cleanup receipt.
struct TerminalTransition {
    result: OpResult,
    _event: Option<EventObservation>,
    _worker: Option<EventObservation>,
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
/// blocks on `cq_wait_checked` is released at the deadline.
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
            let _ = crate::timers::cancel_event(handle).finish();
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
        operation: OperationId,
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
                operation,
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

    fn set_worker_observation(&self, observation: EventObservation) -> ObservationInstallation {
        let mut unused = Some(observation);
        let mut inner = self.inner.lock();
        if matches!(inner.state, OpState::InFlight | OpState::CancelPending) {
            core::mem::swap(&mut inner.worker_observation, &mut unused);
        }
        ObservationInstallation {
            cancellation_won: false,
            unused,
        }
    }

    fn set_timer_observer(
        &self,
        observer: Arc<CompletionTimerObserver>,
        cancel: TimerCancellation,
    ) {
        let mut inner = self.inner.lock();
        inner.timer_observer = Some(observer);
        inner.timer_cancel = Some(cancel);
    }

    /// Returns whether cancellation won before this owner could be installed.
    /// A terminal completion must not retain a late registration.
    pub(crate) fn set_event_observation(
        &self,
        observer: Arc<dyn Observer>,
        registration: crate::klib::observer::registration::Registration<watch_budget::Charge>,
    ) -> ObservationInstallation {
        let mut unused = Some(EventObservation {
            _observer: observer,
            _registration: registration,
        });
        let mut inner = self.inner.lock();
        let cancellation_won = match inner.state {
            OpState::InFlight => {
                core::mem::swap(&mut inner.event_observation, &mut unused);
                false
            }
            OpState::CancelPending => true,
            OpState::Completed(_) | OpState::Observed => false,
        };
        ObservationInstallation {
            cancellation_won,
            unused,
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
    /// effective terminal result and detached subscriptions on the first call,
    /// or `None` if the operation
    /// was already terminal (idempotent). Does **not** signal observers; the
    /// caller wakes waiters after publishing the CQ entry.
    fn complete(&self, result: OpResult) -> Option<TerminalTransition> {
        let mut inner = self.inner.lock();
        let effective = match inner.state {
            OpState::InFlight => result,
            OpState::CancelPending => OpResult::Cancelled,
            OpState::Completed(_) | OpState::Observed => return None,
        };
        inner.state = OpState::Completed(effective.clone());
        Some(TerminalTransition {
            result: effective,
            _event: inner.event_observation.take(),
            _worker: inner.worker_observation.take(),
        })
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
    // Remains here through direct-ring delivery and post-registry disposal;
    // only a retained backlog entry consumes it before the record leaves.
    record_charge: Option<budget::Charge>,
}

struct DetachedTimerObserver {
    asid: AddressSpaceId,
    operation: OperationId,
}

enum BacklogOwner {
    Capability,
    Detached {
        record: RetiredEntry<(OperationId, DetachedOp)>,
    },
}

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
        while let Some(entry) = self.backlog.pop_front() {
            if let BacklogOwner::Detached {
                record,
            } = entry.owner
            {
                dispose_detached_node(record);
            }
        }
    }
}
/// Complete namespace fallback retains all payloads and original accounts.
/// Only explicit post-registry finish may dispose sources, nodes and backing.
#[must_use = "explicitly finish the complete namespace after registry unlock"]
struct AsCompletions(ManuallyDrop<Namespace>);

struct Namespace {
    table: AdmittedMap<CompletionCap, CapabilityRecord>,
    /// Shared upper bound for capability records, in-flight detached
    /// operations, and completed detached records awaiting CQ delivery.
    capacity: usize,
    live: usize,
    /// Live capability-free operations, keyed by their stable operation id.
    /// These count toward `capacity` exactly like capability-backed ones.
    /// Their admitted nodes transfer to terminal/backlog/refund owners.
    detached: AdmittedMap<OperationId, DetachedOp>,
    /// The address space's completion queues, keyed by [`CqId`].
    cqs: AdmittedMap<CqId, CqState>,
    /// Separate lifetime budget: a cancelled timer on another LP stays charged
    /// even after its completion slot has been reclaimed.
    timer_budget: Arc<crate::timers::budget::DomainBudget>,
    record_budget: Arc<budget::DomainBudget>,
    cq_budget: Arc<cq_budget::DomainBudget>,
    watch_budget: Arc<watch_budget::DomainBudget>,
    address_space: Option<crate::memory::AddressSpaceHandle>,
}

impl Deref for AsCompletions {
    type Target = Namespace;

    fn deref(&self) -> &Namespace {
        &self.0
    }
}
impl DerefMut for AsCompletions {
    fn deref_mut(&mut self) -> &mut Namespace {
        &mut self.0
    }
}

impl AsCompletions {
    fn finish(self) {
        let mut entries = ManuallyDrop::into_inner(self.0);
        // Public watches are revoked even if a kernel waiter retains their
        // completion object. Only independent list locks are entered here.
        for completion in entries.table.values() {
            completion.release_event_observation();
            drop(completion.waiters.close());
            drop(completion.detach_callbacks());
        }
        while let Some((&cap, _)) = entries.table.first_key_value() {
            RetiredCapability::from_node(entries.table.take(&cap).unwrap()).finish();
        }
        while let Some((&operation, _)) = entries.detached.first_key_value() {
            dispose_detached_node(entries.detached.take(&operation).unwrap());
        }
        while let Some((&cq, _)) = entries.cqs.first_key_value() {
            RetiredCq::from_node(entries.cqs.take(&cq).unwrap()).finish();
        }
        drop(entries);
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
type Registry = AdmittedMap<AddressSpaceId, AsCompletions>;
static COMPLETIONS: LazyLock<RwLock<Registry>> = LazyLock::new(|| RwLock::new(Registry::new()));

// Mandatory private boot fixture; runtime publication uses fallible Preparing.
fn empty_as(asid: AddressSpaceId, capacity: usize) -> AsCompletions {
    try_empty_as(crate::memory::current_address_space_handle(asid), capacity, &mut |_| Ok(()))
        .expect("mandatory completion namespace allocation failed")
}

fn try_empty_as(
    root: Option<crate::memory::AddressSpaceHandle>,
    capacity: usize,
    before_allocate: &mut impl FnMut(u8) -> Result<(), core::alloc::AllocError>,
) -> Result<AsCompletions, core::alloc::AllocError> {
    let capacity = capacity.min(budget::MAX_DOMAIN_RECORDS);
    // No payload/source/authority is published. Partial account construction
    // releases only unused Arcs outside local registry/lifecycle guards.
    before_allocate(1)?;
    let timer_budget = crate::timers::budget::DomainBudget::try_new(capacity)?;
    before_allocate(2)?;
    let record_budget = budget::DomainBudget::try_new(capacity)?;
    before_allocate(3)?;
    let cq_budget = cq_budget::DomainBudget::try_new()?;
    before_allocate(4)?;
    let watch_budget = watch_budget::DomainBudget::try_new(capacity)?;
    Ok(AsCompletions(ManuallyDrop::new(Namespace {
        table: AdmittedMap::new(),
        capacity,
        live: 0,
        detached: AdmittedMap::new(),
        cqs: AdmittedMap::new(),
        timer_budget,
        record_budget,
        cq_budget,
        watch_budget,
        address_space: root,
    })))
}

/// The caller retains both the complete detached namespace and COMPLETIONS
/// through authority removal. Destroy payload/source metadata after unlock.
fn revoke_namespace_authority(asid: AddressSpaceId, previous: &mut AsCompletions) {
    for (cap, record) in previous.table.iter_mut() {
        record.revoke(asid, *cap);
    }
}

/// Mandatory kernel/demo/serialized-fixture adapter. Runtime domain setup uses
/// the fallible CQ namespace entry points below; all share prepared storage.
pub fn open_address_space(asid: AddressSpaceId, capacity: usize) {
    namespace_preparation::open(asid, capacity)
        .expect("mandatory completion namespace publication failed");
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
    registry: &Registry,
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

fn open_namespace_with_cq(
    asid: AddressSpaceId,
    capacity: usize,
    cq_entries: u32,
    frame: Option<crate::memory::physical::PAddr>,
) -> Result<(), CqOpenError> {
    queue_preparation::open_namespace(asid, capacity, cq_entries, frame)
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
    queue_preparation::attach(asid, cq, cq_entries, frame)
}

/// Kernel-controlled queue replacement discards old undelivered results.
/// Retained detached results must return their submission slots as well as
/// their owning record charges; capability results still live in the table.
fn replace_cq(
    entries: &mut AsCompletions,
    cq: CqId,
    node: PreparedEntry<(CqId, CqState)>,
    replacement: CqState,
) -> Option<RetiredCq> {
    let previous = entries.cqs.take(&cq);
    entries.cqs.insert(node, cq, replacement);
    if let Some(previous) = &previous {
        let discarded = previous
            .value()
            .1
            .backlog
            .iter()
            .filter(|entry| matches!(&entry.owner, BacklogOwner::Detached { .. }))
            .count();
        entries.live = entries
            .live
            .checked_sub(discarded)
            .expect("discarded detached records must count as live");
    }
    previous.map(RetiredCq::from_node)
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
    let retired = {
        let mut registry = COMPLETIONS.write();
        let mut retired = registry.take(&asid).map(RetiredNamespace::from_node);
        if let Some(retired) = &mut retired {
            revoke_namespace_authority(asid, retired);
        }
        retired
    };
    if let Some(retired) = retired {
        retired.finish();
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
    Ok(Arc::clone(completion))
}

/// Starts an async operation. Returns immediately with a capability naming it;
/// ownership of `buffer` transfers to the kernel until a terminal completion is
/// posted. Returns [`SubmitError::WouldBlock`] under submission backpressure.
pub fn submit(
    asid: AddressSpaceId,
    _op: OpCode,
    buffer: Option<Vec<u8>>,
) -> Result<CompletionCap, SubmitError> {
    submit_captured(asid, buffer, false, &OPERATION_IDS).map(|(cap, _, _)| cap)
}

fn submit_captured(
    asid: AddressSpaceId,
    buffer: Option<Vec<u8>>,
    event_watch: bool,
    ids: &operation_ids::OperationIds,
) -> Result<(CompletionCap, CompletionRef, Option<watch_budget::Charge>), SubmitError> {
    capability_preparation::submit(asid, buffer, event_watch, ids)
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
        let (cap, completion, charge) = submit_captured(asid, None, true, &OPERATION_IDS)?;
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
        let installation = {
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
                })
            } else {
                self.completion.set_event_observation(observer, registration)
            }
        };
        Ok(installation.finish())
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
        let retired = {
            let mut registry = COMPLETIONS.write();
            if let Some(entries) = registry.get_mut(&self.asid)
                && entries
                    .table
                    .get(&self.cap)
                    .is_some_and(|live| Arc::ptr_eq(live, &self.completion))
            {
                Some(capability_metadata::retire(entries, self.asid, self.cap).unwrap())
            } else {
                None
            }
        };
        self.completion.release_event_observation();
        drop(self.completion.detach_callbacks());
        if let Some(retired) = retired {
            retired.finish();
        }
    }
}

/// Roll back a capability-backed submission before it becomes externally
/// visible. Retain both metadata owners and remove any terminal backlog row
/// before returning its slot, then dispose outside the local registry hold.
pub(crate) fn abort_submission(asid: AddressSpaceId, cap: CompletionCap) -> Result<(), CapError> {
    let retired = {
        let mut registry = COMPLETIONS.write();
        let entries = registry.get_mut(&asid).ok_or(CapError::UnknownAddressSpace)?;
        capability_metadata::retire(entries, asid, cap)?
    };
    retired.completion().release_event_observation();
    drop(retired.completion().detach_callbacks());
    retired.finish();
    Ok(())
}

/// Submits a timer operation: creates a capability that auto-completes after
/// `timeout_ms` milliseconds. The returned cap delivers a completion ring entry
/// when the deadline expires, so a user-space service waiting on `cq_wait_checked` is
/// released exactly at the deadline.
pub fn submit_timer(asid: AddressSpaceId, timeout_ms: u64) -> Result<CompletionCap, SubmitError> {
    submit_timer_with_observer(asid, timeout_ms, &OPERATION_IDS, Arc::try_new)
}

fn submit_timer_with_observer(
    asid: AddressSpaceId,
    timeout_ms: u64,
    ids: &operation_ids::OperationIds,
    allocate: impl FnOnce(
        CompletionTimerObserver,
    ) -> Result<Arc<CompletionTimerObserver>, core::alloc::AllocError>,
) -> Result<CompletionCap, SubmitError> {
    capability_preparation::submit_timer(asid, timeout_ms, ids, allocate)
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
    let mut releases = budget::Releases::default();
    let (transition, cq_source) = {
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
        let Some(transition) = completion.complete(result) else {
            // Already terminal: idempotent no-op, no duplicate CQ entry.
            return Ok(());
        };
        let mut cq_source = None;
        if let Some(as_completions) = registry.get_mut(&asid).map(|entries| &mut **entries)
            && as_completions
                .table
                .get(&cap)
                .is_some_and(|registered| Arc::ptr_eq(registered, &completion))
            && let Some(cq_state) = as_completions.cqs.get_mut(&DEFAULT_CQ)
        {
            let op = completion.operation_id();
            let flushed_detached = post_to_cq(
                cq_state,
                op,
                cap,
                &transition.result,
                || BacklogOwner::Capability,
                &mut releases,
            )
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
            cq_source = Some(cq_state.waiters.clone());
        }
        (transition, cq_source)
    };

    // Destructors may reenter or replace the namespace. Carry the exact CQ
    // source from publication so disposal cannot redirect this wake to a new
    // namespace/queue that happens to reuse the same numeric identifiers.
    drop(releases);
    drop(transition);
    if let Some(source) = cq_source {
        signal_cq_source(asid, DEFAULT_CQ, &source);
    }
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
    owner: impl FnOnce() -> BacklogOwner,
    releases: &mut budget::Releases,
) -> PostOutcome {
    let (status, val) = crate::completion::cq::op_result_to_fields(result);
    let flushed = flush_backlog(cq_state, releases);
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
            // Transfer admission only if a retained entry needs it. A direct
            // delivery leaves its existing record owner intact for disposal
            // beyond the containing registry guard.
            owner: owner(),
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
    detached_preparation::submit(asid, cq, user_data, &OPERATION_IDS)
}

/// Submit a timer delivered exclusively through a completion queue.
pub fn submit_detached_timer(
    asid: AddressSpaceId,
    cq: CqId,
    timeout_ms: u64,
    user_data: u64,
) -> Result<OperationId, SubmitError> {
    submit_detached_timer_with_observer(asid, cq, timeout_ms, user_data, Arc::try_new)
}

fn submit_detached_timer_with_observer(
    asid: AddressSpaceId,
    cq: CqId,
    timeout_ms: u64,
    user_data: u64,
    allocate: impl FnOnce(
        DetachedTimerObserver,
    ) -> Result<Arc<DetachedTimerObserver>, core::alloc::AllocError>,
) -> Result<OperationId, SubmitError> {
    detached_preparation::submit_timer(asid, cq, timeout_ms, user_data, &OPERATION_IDS, allocate)
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
    retire_detached(asid, operation, result)?.finish();
    Ok(())
}

/// Complete logical terminal retirement, not a physical cleanup receipt.
/// Keep every remaining record field and the exact published CQ source until
/// producer disposal outside COMPLETIONS, before any notification.
#[must_use = "dispose the complete detached record after registry unlock, then notify its captured \
              source"]
struct RetiredDetached(ManuallyDrop<DetachedRetirement>);

struct DetachedRetirement {
    asid: AddressSpaceId,
    record: Option<RetiredEntry<(OperationId, DetachedOp)>>,
    cq: CqId,
    observer: Option<Arc<DetachedTimerObserver>>,
    cancel: Option<TimerCancellation>,
    releases: budget::Releases,
    source: Option<
        crate::klib::observer::registration::ListRef<crate::klib::observer::waiter_budget::Charge>,
    >,
}

impl RetiredDetached {
    fn finish(self) {
        let DetachedRetirement {
            asid,
            mut record,
            cq,
            observer,
            cancel,
            releases,
            source,
        } = ManuallyDrop::into_inner(self.0);
        drop(releases);
        let charge = record.as_mut().and_then(|node| node.value_mut().1.record_charge.take());
        if let Some(record) = record {
            record.release();
        }
        drop(observer);
        drop(cancel);
        drop(charge);
        if let Some(source) = source {
            signal_cq_source(asid, cq, &source);
        }
    }
}

/// Retain the original record charge through actual admitted-node release.
/// Backlogged records have already moved that charge into their CQ owner.
fn dispose_detached_node(mut record: RetiredEntry<(OperationId, DetachedOp)>) {
    let charge = record.value_mut().1.record_charge.take();
    record.release();
    drop(charge);
}

fn retire_detached(
    asid: AddressSpaceId,
    operation: OperationId,
    result: OpResult,
) -> Result<RetiredDetached, CapError> {
    let retired = {
        let mut registry = COMPLETIONS.write();
        let as_completions = registry.get_mut(&asid).ok_or(CapError::UnknownAddressSpace)?;
        let detached = as_completions.detached.take(&operation).ok_or(CapError::UnknownCap)?;
        post_detached(asid, operation, result, as_completions, detached)
    };
    Ok(retired)
}

/// The caller serializes removal and this splice under COMPLETIONS. Returning
/// the complete remaining record keeps timer and direct-result cleanup out of
/// that hold. A backlog takes the admitted node and its original charge;
/// direct delivery retains it in this receipt until post-guard disposal.
fn post_detached(
    asid: AddressSpaceId,
    operation: OperationId,
    result: OpResult,
    as_completions: &mut Namespace,
    mut record: RetiredEntry<(OperationId, DetachedOp)>,
) -> RetiredDetached {
    let detached = &mut record.value_mut().1;
    let cq = detached.cq;
    let user_data = detached.user_data;
    let observer = detached._timer_observer.take();
    let cancel = detached.timer_cancel.take();
    let mut releases = budget::Releases::default();
    let effective = if detached.cancel_pending {
        OpResult::Cancelled
    } else {
        result
    };
    let mut record = Some(record);
    let source = if let Some(cq_state) = as_completions.cqs.get_mut(&cq) {
        let outcome = post_to_cq(
            cq_state,
            operation,
            user_data,
            &effective,
            || BacklogOwner::Detached {
                record: record.take().expect("detached record queued twice"),
            },
            &mut releases,
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
        Some(cq_state.waiters.clone())
    } else {
        // The queue disappeared after submission, so no delivery can be
        // retained. Release this operation's submission slot.
        as_completions.live = as_completions.live.saturating_sub(1);
        None
    };
    RetiredDetached(ManuallyDrop::new(DetachedRetirement {
        asid,
        record,
        cq,
        observer,
        cancel,
        releases,
        source,
    }))
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
fn flush_backlog(cq_state: &mut CqState, releases: &mut budget::Releases) -> FlushOutcome {
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
        if let BacklogOwner::Detached {
            mut record,
        } = entry.owner
        {
            outcome.detached += 1;
            releases.push(record.value_mut().1.record_charge.take().expect("backlog lost charge"));
            releases.push_node(record);
        }
    }
    if outcome.total != 0 {
        // Retained records becoming visible are new work even if the original
        // completion generation was consumed before ring space was available.
        cq_state.work_generation = cq_state.work_generation.wrapping_add(1);
    }
    outcome
}

fn flush_cq_backlog(
    as_completions: &mut AsCompletions,
    cq: CqId,
    releases: &mut budget::Releases,
) -> FlushOutcome {
    let outcome = as_completions
        .cqs
        .get_mut(&cq)
        .map(|state| flush_backlog(state, releases))
        .unwrap_or_default();
    as_completions.live = as_completions.live.saturating_sub(outcome.detached);
    outcome
}

fn signal_cq_source(
    asid: AddressSpaceId,
    cq: CqId,
    source: &crate::klib::observer::registration::ListRef<
        crate::klib::observer::waiter_budget::Charge,
    >,
) {
    let count = source.registered();
    let waiters = source.drain();
    waiters.notify();
    crate::debug_trace::trace(
        crate::debug_trace::TAG_SIGNAL_CQ,
        asid as u64,
        cq as u64,
        count as u64,
    );
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
        let setup = crate::cpu::scheduler::prepare_park();
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
        drop(setup);
        yield_lp();
        // A competing wake is not proof that the producer has terminated.
    }
    Ok(())
}

/// Blocks the calling thread until `cap` reaches a terminal completion or
/// `timeout_ms` elapses. Returns `true` if terminal, `false` only on deadline
/// expiry, and `WaitAdmissionFailed` on waiter/watchdog rejection. Neither
/// rejection nor timeout consumes the capability or its buffer.
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
    crate::cpu::scheduler::try_block_until(
        completion.as_ref() as &dyn Observable,
        timeout_ms,
        || completion.is_terminal(),
    )
    .map_err(|_| CapError::WaitAdmissionFailed)
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
    retire_completed(asid, cap, &completion)?.finish();
    Ok(())
}

/// Exact-object logical retirement; the returned owner retains payload and
/// authority metadata until explicit post-guard release, without retry policy.
fn retire_completed(
    asid: AddressSpaceId,
    cap: CompletionCap,
    completion: &CompletionRef,
) -> Result<RetiredCapability, CapError> {
    if !completion.is_reclaimable() {
        return Err(CapError::NotComplete);
    }
    let mut registry = COMPLETIONS.write();
    let entries = registry.get_mut(&asid).ok_or(CapError::UnknownAddressSpace)?;
    if !entries.table.get(&cap).is_some_and(|live| Arc::ptr_eq(live, completion)) {
        return Err(CapError::UnknownCap);
    }
    capability_metadata::retire(entries, asid, cap)
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
    // Declared before the guard: even an early return releases charges only
    // after registry unlock. The batch retains exact original accounts/classes.
    let mut releases = budget::Releases::default();
    let mut registry = COMPLETIONS.write();
    let Some(as_completions) = registry.get_mut(&asid) else {
        return 0;
    };
    flush_cq_backlog(as_completions, cq, &mut releases);
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
/// [`wait_on_cq_checked`] returns even though no completion
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

/// A wake destination captured from one exact CQ lifetime. Retaining its
/// admitted waiter-list identity prevents an old callback from waking a
/// successor at the same numeric ASID/CQ; it does not retain ring backing.
#[derive(Debug, Clone)]
pub(crate) struct CqInterest {
    asid: AddressSpaceId,
    cq: CqId,
    identity:
        crate::klib::observer::registration::ListRef<crate::klib::observer::waiter_budget::Charge>,
}

pub(crate) fn capture_cq_interest(asid: AddressSpaceId, cq: CqId) -> Option<CqInterest> {
    let registry = COMPLETIONS.read();
    let state = registry.get(&asid)?.cqs.get(&cq)?;
    Some(CqInterest {
        asid,
        cq,
        identity: state.waiters.clone(),
    })
}

impl CqInterest {
    pub(crate) fn wake(&self) {
        let notification = {
            let mut registry = COMPLETIONS.write();
            let Some(state) =
                registry.get_mut(&self.asid).and_then(|entries| entries.cqs.get_mut(&self.cq))
            else {
                return;
            };
            if !core::ptr::eq(&*state.waiters, &*self.identity) {
                return;
            }
            state.work_generation = state.work_generation.wrapping_add(1);
            state.waiters.drain()
        };
        notification.notify();
    }
}

impl CqInterest {
    fn work_pending(&self) -> Option<bool> {
        let registry = COMPLETIONS.read();
        let state = registry.get(&self.asid)?.cqs.get(&self.cq)?;
        core::ptr::eq(&*state.waiters, &*self.identity)
            .then_some(state.work_generation != state.last_seen_generation)
    }

    fn publish_waiter(
        &self,
        prepared: &mut crate::klib::observer::PreparedWaitRegistration,
    ) -> Result<
        crate::klib::observer::WaitRegistration,
        crate::klib::observer::registration::RegistrationError,
    > {
        let registry = COMPLETIONS.read();
        let state = registry
            .get(&self.asid)
            .and_then(|entries| entries.cqs.get(&self.cq))
            .filter(|state| {
                prepared.matches(&state.waiters) && core::ptr::eq(&*state.waiters, &*self.identity)
            })
            .ok_or(crate::klib::observer::registration::RegistrationError::Closed)?;
        if state.work_generation != state.last_seen_generation {
            Ok(crate::klib::observer::WaitRegistration::ready())
        } else {
            prepared.publish()
        }
    }
}

impl Observable for CqInterest {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &crate::klib::observer::WaitSponsor,
    ) -> Result<
        crate::klib::observer::WaitRegistration,
        crate::klib::observer::registration::RegistrationError,
    > {
        match self.work_pending() {
            None => Err(crate::klib::observer::registration::RegistrationError::Closed),
            Some(true) => Ok(crate::klib::observer::WaitRegistration::ready()),
            Some(false) => {
                let mut prepared = sponsor.prepare(&self.identity, observer)?;
                self.publish_waiter(&mut prepared)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CqWaitError {
    UnknownQueue,
    AdmissionRejected,
}

/// Wait on one captured CQ lifetime. None is event-only; Some uses the
/// scheduler's owned watchdog without allocating a completion capability.
/// Spurious wakes repark within the original absolute deadline.
pub fn wait_on_cq_checked(
    asid: AddressSpaceId,
    cq: CqId,
    timeout_ms: Option<u64>,
) -> Result<bool, CqWaitError> {
    let mut releases = budget::Releases::default();
    let source = {
        let mut registry = COMPLETIONS.write();
        let entries = registry.get_mut(&asid).ok_or(CqWaitError::UnknownQueue)?;
        flush_cq_backlog(entries, cq, &mut releases);
        let state = entries.cqs.get(&cq).ok_or(CqWaitError::UnknownQueue)?;
        CqInterest {
            asid,
            cq,
            identity: state.waiters.clone(),
        }
    };
    drop(releases); // Refund before preparing/parking on the captured interest.
    let condition = || source.work_pending().unwrap_or(true);
    let constraint = crate::cpu::scheduler::threads::MigrationConstraint::CompletionQueueWait;
    let waited = match timeout_ms {
        Some(timeout) => crate::cpu::scheduler::try_block_until_with_constraint(
            &source, timeout, constraint, condition,
        ),
        None => {
            crate::cpu::scheduler::try_park_until(&source, constraint, condition).map(|()| true)
        }
    };
    let mut registry = COMPLETIONS.write();
    let state = registry
        .get_mut(&asid)
        .and_then(|entries| entries.cqs.get_mut(&cq))
        .ok_or(CqWaitError::UnknownQueue)?;
    if !core::ptr::eq(&*state.waiters, &*source.identity) {
        return Err(CqWaitError::UnknownQueue);
    }
    if state.work_generation != state.last_seen_generation {
        state.last_seen_generation = state.work_generation;
        Ok(true)
    } else {
        waited.map_err(|_| CqWaitError::AdmissionRejected)
    }
}

/// Kernel fixture: a detached wake batch belongs to the captured CQ, not a
/// later namespace occupying the same numeric ASID/CQ. Reentrant callbacks
/// also exercise the rule that no registry/device guard survives notification.
pub(crate) fn test_prepared_wake_identity() {
    waiter_tests::run();
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
    let old_registration = capture_cq_interest(CLIENT, 0)
        .unwrap()
        .try_register_waiter(Arc::downgrade(&old), &sponsor)
        .unwrap();
    let stale_interest = capture_cq_interest(CLIENT, 0).unwrap();
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
    let new_registration = capture_cq_interest(CLIENT, 0)
        .unwrap()
        .try_register_waiter(Arc::downgrade(&new), &sponsor)
        .unwrap();
    stale_interest.wake();
    assert_eq!(cq_work_generation(CLIENT, 0), 0);
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
    // Fixture an already waiting reader independently of the queue's
    // last_seen cursor; this pseudo domain has no scheduled reactor. Capture
    // under the registry, then allocate outside it like ordinary waiters.
    let source = capture_cq_interest(asid, 0).unwrap();
    let registration = sponsor.register(&source.identity, Arc::downgrade(&callback)).unwrap();
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
