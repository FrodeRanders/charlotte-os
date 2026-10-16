//! Detached record/timer preparation outside COMPLETIONS. Capture/publication
//! retain backlog reconciliation; accepted timers enqueue once after unlock.

use core::mem::ManuallyDrop;

use super::*;

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
        drop(resources);
    }
}

pub(super) fn submit(
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

pub(super) fn submit_timer(
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
mod tests;

pub(super) fn test_preparation() {
    tests::run();
    metadata_tests::run();
}
