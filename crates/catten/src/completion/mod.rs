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
mod capability_metadata;
use capability_metadata::{
    CapabilityRecord,
    RetiredCapability,
};
pub(crate) use record::callback_tests;
pub use record::{
    Completion,
    CompletionObservation,
    ObserveError,
    cancel,
    observe,
};
pub mod cq;
pub(crate) mod cq_budget;
mod delivery;
use delivery::{
    BacklogOwner,
    CqState,
    FlushOutcome,
    RetiredCq,
    signal_cq_source,
};
pub(crate) use delivery::{
    CqInterest,
    capture_cq_interest,
    cq_waiter_count,
    prepare_wake,
    test_irq_wake_reentrancy,
    test_prepared_wake_identity,
};
pub use delivery::{
    CqWaitError,
    cq_pending,
    cq_work_generation,
    wait_on_cq_checked,
    wake,
};
mod detached;
use detached::{
    DetachedOp,
    dispose_detached_node,
};
pub use detached::{
    cancel_detached,
    complete_detached,
    submit_detached,
    submit_detached_timer,
};
mod namespace;
use namespace::{
    Namespace,
    NamespaceState,
    Registry,
    RetiredNamespace,
};
pub use namespace::{
    close_address_space,
    open_address_space,
};
mod operation_id_tests;
mod operation_ids;
mod producers;
mod record;
use producers::{
    CompletionTimerObserver,
    TimerCancellation,
};
pub(crate) use producers::{
    EventSubmission,
    abort_submission,
    exit_tests,
    observe_own_thread_exit,
    observe_thread_exit_with_generation,
    timer_tests,
};
pub use producers::{
    submit_timer,
    submit_worker,
};
pub(crate) mod watch_budget;

use alloc::{
    sync::{
        Arc,
        Weak,
    },
    vec::Vec,
};
use core::mem::ManuallyDrop;

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

/// Identifies one completion queue within an address space (architecture doc
/// §8.1: one CQ per shard). Queue 0 ([`DEFAULT_CQ`]) always exists when any
/// CQ is attached and is the destination for capability-backed completions
/// and for callers that do not select a queue.
pub type CqId = u32;

/// The default completion queue of an address space.
pub const DEFAULT_CQ: CqId = 0;

/// Per-address-space capability tables. In a full design these would live inside
/// each `AddressSpace`; keying them here keeps the prototype self-contained
/// without modifying the address-space type.
static COMPLETIONS: LazyLock<RwLock<Registry>> = LazyLock::new(|| RwLock::new(Registry::new()));

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
                .any(|(id, state)| state.ring_ptr() == ptr && (*owner != asid || *id != cq))
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
    delivery::open_namespace(asid, capacity, cq_entries, frame)
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
    delivery::attach(asid, cq, cq_entries, frame)
}

/// Kernel-controlled queue replacement discards old undelivered results.
/// Retained detached results must return their submission slots as well as
/// their owning record charges; capability results still live in the table.
fn replace_cq(
    entries: &mut Namespace,
    cq: CqId,
    node: PreparedEntry<(CqId, CqState)>,
    replacement: CqState,
) -> Option<RetiredCq> {
    let previous = entries.cqs.take(&cq);
    entries.cqs.insert(node, cq, replacement);
    if let Some(previous) = &previous {
        let discarded = previous.value().1.detached_backlog_len();
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
    registry.get(&asid).and_then(|c| c.cqs.get(&cq)).map(|state| state.ring_ptr())
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
    record::submit_with_ids(asid, buffer, event_watch, ids)
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
            let flushed_detached = cq_state
                .post(op, cap, &transition.result, || BacklogOwner::Capability, &mut releases)
                .flushed
                .detached;
            as_completions.live = as_completions.live.saturating_sub(flushed_detached);
            let generation = cq_state.advance_work();
            crate::debug_trace::trace(
                crate::debug_trace::TAG_COMPLETE,
                asid as u64,
                generation,
                cap,
            );
            cq_source = Some(cq_state.waiter_source().clone());
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

fn flush_cq_backlog(
    as_completions: &mut Namespace,
    cq: CqId,
    releases: &mut budget::Releases,
) -> FlushOutcome {
    let outcome =
        as_completions.cqs.get_mut(&cq).map(|state| state.flush(releases)).unwrap_or_default();
    as_completions.live = as_completions.live.saturating_sub(outcome.detached);
    outcome
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

pub(crate) fn timer_events_used(asid: AddressSpaceId) -> usize {
    COMPLETIONS.read().get(&asid).map_or(0, |entries| entries.timer_budget.used())
}

pub(crate) fn waiter_count(asid: AddressSpaceId, cap: CompletionCap) -> usize {
    completion_of(asid, cap).map_or(0, |completion| completion.waiters.registered())
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

/// Close the captured object while leaving any successor with the same handle untouched.
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
