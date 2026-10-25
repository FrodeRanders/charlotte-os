//! Capability-free completion lifecycle: fallible preparation, one-time
//! publication, producer completion/cancellation and post-registry retirement.
//!
//! Captured target/source identity, timer custody, storage and original charges
//! remain in their complete owners. Backlog delivery still uses the common CQ
//! path; readiness grants no cleanup/replay authority or new caller-context proof.
use core::mem::ManuallyDrop;

use super::*;

/// A capability-free operation (architecture doc §8.4): tracked only by its
/// [`OperationId`], delivered exclusively through the CQ ring, and reclaimed
/// on completion. The operation ID then stops being addressable, but an
/// undelivered CQ result retains record admission until publication/discard.
pub(super) struct DetachedOp {
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
    pub(super) record_charge: Option<budget::Charge>,
}

pub(super) struct DetachedTimerObserver {
    asid: AddressSpaceId,
    operation: OperationId,
}

impl Observer for DetachedTimerObserver {
    fn notify(self: Arc<Self>) {
        let _ = complete_detached(self.asid, self.operation, OpResult::Ok(0));
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
    submit_with_ids(asid, cq, user_data, &OPERATION_IDS)
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

pub(super) fn submit_detached_timer_with_observer(
    asid: AddressSpaceId,
    cq: CqId,
    timeout_ms: u64,
    user_data: u64,
    allocate: impl FnOnce(
        DetachedTimerObserver,
    ) -> Result<Arc<DetachedTimerObserver>, core::alloc::AllocError>,
) -> Result<OperationId, SubmitError> {
    submit_timer_with_ids(asid, cq, timeout_ms, user_data, &OPERATION_IDS, allocate)
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

/// Own the complete logically retired record. Physical cleanup needs its own completion proof.
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
pub(super) fn dispose_detached_node(mut record: RetiredEntry<(OperationId, DetachedOp)>) {
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
        let outcome = cq_state.post(
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

type Source =
    crate::klib::observer::registration::ListRef<crate::klib::observer::waiter_budget::Charge>;

struct Target {
    asid: AddressSpaceId,
    cq: CqId,
    root: Option<crate::memory::AddressSpaceHandle>,
    records: Arc<budget::DomainBudget>,
    timers: Arc<crate::timers::budget::DomainBudget>,
    capacity: usize,
    platform: bool,
    source: Source,
}

impl Target {
    fn capture(asid: AddressSpaceId, cq: CqId) -> Result<Self, SubmitError> {
        let platform = crate::memory::budget::platform_identity(asid);
        // The refund owner outlives this local registry guard on every path.
        let mut releases = budget::Releases::default();
        let mut registry = COMPLETIONS.write();
        let entries = registry.get_mut(&asid).ok_or(SubmitError::UnknownAddressSpace)?;
        if entries.address_space.is_some_and(|root| !crate::memory::budget::accepting(root)) {
            return Err(SubmitError::UnknownAddressSpace);
        }
        flush_cq_backlog(entries, cq, &mut releases);
        let queue = entries.cqs.get(&cq).ok_or(SubmitError::NoCompletionQueue)?;
        if entries.live >= entries.capacity {
            return Err(SubmitError::WouldBlock);
        }
        Ok(Self {
            asid,
            cq,
            root: entries.address_space,
            records: entries.record_budget.clone(),
            timers: entries.timer_budget.clone(),
            capacity: entries.capacity,
            platform: asid == crate::memory::KERNEL_ASID
                || platform.is_some_and(|root| Some(root) == entries.address_space),
            source: queue.waiters.clone(),
        })
    }

    fn validate(&self, entries: &AsCompletions) -> Result<(), SubmitError> {
        if self.root != entries.address_space
            || !Arc::ptr_eq(&self.records, &entries.record_budget)
            || !Arc::ptr_eq(&self.timers, &entries.timer_budget)
            || self.capacity != entries.capacity
            || self.root.is_some_and(|root| !crate::memory::budget::accepting(root))
        {
            return Err(SubmitError::UnknownAddressSpace);
        }
        if !entries
            .cqs
            .get(&self.cq)
            .is_some_and(|queue| core::ptr::eq(&*queue.waiters, &*self.source))
        {
            return Err(SubmitError::NoCompletionQueue);
        }
        Ok(())
    }
}

/// Own the exact target, original record/timer admission and unpublished node
/// through every preparation failure or rejected borrowed publication.
#[must_use]
struct Preparing(ManuallyDrop<Resources>);

struct Resources {
    target: Target,
    operation: OperationId,
    record: Option<DetachedOp>,
    entry: Option<PreparedEntry<(OperationId, DetachedOp)>>,
    event: Option<crate::timers::PreparedEvent>,
}

impl Preparing {
    fn plain(
        target: Target,
        user_data: u64,
        ids: &operation_ids::OperationIds,
    ) -> Result<Self, SubmitError> {
        Self::plain_with(target, user_data, ids, PreparedEntry::try_new)
    }

    fn plain_with(
        target: Target,
        user_data: u64,
        ids: &operation_ids::OperationIds,
        allocate: impl FnOnce() -> Result<
            PreparedEntry<(OperationId, DetachedOp)>,
            core::alloc::AllocError,
        >,
    ) -> Result<Self, SubmitError> {
        let operation = ids.allocate().ok_or(SubmitError::WouldBlock)?;
        let mut preparing = Self(ManuallyDrop::new(Resources {
            target,
            operation,
            record: None,
            entry: None,
            event: None,
        }));
        let result = (|| {
            let resources = &mut *preparing.0;
            let charge = budget::reserve(&resources.target.records, resources.target.platform)
                .map_err(|_| SubmitError::WouldBlock)?;
            resources.record = Some(DetachedOp {
                user_data,
                cq: resources.target.cq,
                cancel_pending: false,
                _timer_observer: None,
                timer_cancel: None,
                record_charge: Some(charge),
            });
            resources.entry = Some(allocate().map_err(|_| SubmitError::WouldBlock)?);
            Ok(())
        })();
        if let Err(error) = result {
            preparing.finish();
            return Err(error);
        }
        Ok(preparing)
    }

    fn timer(
        target: Target,
        timeout_ms: u64,
        user_data: u64,
        ids: &operation_ids::OperationIds,
        allocate: impl FnOnce(
            DetachedTimerObserver,
        ) -> Result<Arc<DetachedTimerObserver>, core::alloc::AllocError>,
    ) -> Result<Self, SubmitError> {
        Self::timer_with(
            target,
            timeout_ms,
            user_data,
            ids,
            crate::timers::PreparedEvent::new,
            allocate,
        )
    }

    fn timer_with(
        target: Target,
        timeout_ms: u64,
        user_data: u64,
        ids: &operation_ids::OperationIds,
        allocate_event: impl FnOnce(TimerEvent) -> Result<crate::timers::PreparedEvent, ()>,
        allocate: impl FnOnce(
            DetachedTimerObserver,
        ) -> Result<Arc<DetachedTimerObserver>, core::alloc::AllocError>,
    ) -> Result<Self, SubmitError> {
        let mut preparing = Self::plain(target, user_data, ids)?;
        let result = (|| {
            let charge = crate::timers::budget::reserve(
                &preparing.0.target.timers,
                preparing.0.target.platform,
            )
            .map_err(|_| SubmitError::WouldBlock)?;
            let (event, cancel) =
                TimerEvent::charged(ExtDuration::from_millis(timeout_ms as u128), charge)
                    .map_err(|_| SubmitError::WouldBlock)?;
            preparing.0.record.as_mut().unwrap().timer_cancel =
                Some(TimerCancellation(Some(cancel)));
            preparing.0.event = Some(allocate_event(event).map_err(|_| SubmitError::WouldBlock)?);
            let observer = allocate(DetachedTimerObserver {
                asid: preparing.0.target.asid,
                operation: preparing.0.operation,
            })
            .map_err(|_| SubmitError::WouldBlock)?;
            preparing
                .0
                .event
                .as_ref()
                .unwrap()
                .event()
                .register_observer(Arc::downgrade(&observer) as Weak<dyn Observer>);
            preparing.0.record.as_mut().unwrap()._timer_observer = Some(observer);
            Ok(())
        })();
        if let Err(error) = result {
            preparing.finish();
            return Err(error);
        }
        Ok(preparing)
    }

    fn publish(&mut self) -> Result<OperationId, SubmitError> {
        let mut releases = budget::Releases::default();
        let mut registry = COMPLETIONS.write();
        let resources = &mut *self.0;
        let entries =
            registry.get_mut(&resources.target.asid).ok_or(SubmitError::UnknownAddressSpace)?;
        resources.target.validate(entries)?;
        flush_cq_backlog(entries, resources.target.cq, &mut releases);
        if entries.live >= entries.capacity {
            return Err(SubmitError::WouldBlock);
        }
        entries.detached.insert(
            resources.entry.take().expect("detached preparation published twice"),
            resources.operation,
            resources.record.take().expect("detached preparation lost record"),
        );
        entries.live += 1;
        Ok(resources.operation)
    }

    fn enqueue(&mut self) {
        assert!(self.0.record.is_none(), "unpublished detached timer cannot enqueue");
        if let Some(event) = self.0.event.take() {
            event.enqueue();
        }
    }

    fn finish(self) {
        let mut resources = ManuallyDrop::into_inner(self.0);
        // Empty prepared storage is freed before its original record charge.
        drop(resources.entry.take());
        if let Some(event) = resources.event.take() {
            event.cancel_unpublished();
        }
        drop(resources);
    }
}

pub(super) fn submit_with_ids(
    asid: AddressSpaceId,
    cq: CqId,
    user_data: u64,
    ids: &operation_ids::OperationIds,
) -> Result<OperationId, SubmitError> {
    let mut preparing = Preparing::plain(Target::capture(asid, cq)?, user_data, ids)?;
    let result = preparing.publish();
    if result.is_ok() {
        preparing.enqueue();
    }
    preparing.finish();
    result
}

pub(super) fn submit_timer_with_ids(
    asid: AddressSpaceId,
    cq: CqId,
    timeout_ms: u64,
    user_data: u64,
    ids: &operation_ids::OperationIds,
    allocate: impl FnOnce(
        DetachedTimerObserver,
    ) -> Result<Arc<DetachedTimerObserver>, core::alloc::AllocError>,
) -> Result<OperationId, SubmitError> {
    let mut preparing =
        Preparing::timer(Target::capture(asid, cq)?, timeout_ms, user_data, ids, allocate)?;
    let result = preparing.publish();
    if result.is_ok() {
        preparing.enqueue();
    }
    preparing.finish();
    result
}

mod metadata_tests;
mod preparation_tests;
mod retirement_tests;

pub(super) fn test_lifecycle() {
    retirement_tests::run();
    preparation_tests::run();
    metadata_tests::run();
}
