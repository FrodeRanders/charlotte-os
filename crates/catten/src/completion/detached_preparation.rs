//! Detached record/timer preparation outside COMPLETIONS. Capture/publication
//! retain backlog reconciliation; accepted timers enqueue once after unlock.

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
        let mut registry = COMPLETIONS.write();
        let entries = registry.get_mut(&asid).ok_or(SubmitError::UnknownAddressSpace)?;
        if entries.address_space.is_some_and(|root| !crate::memory::budget::accepting(root)) {
            return Err(SubmitError::UnknownAddressSpace);
        }
        flush_cq_backlog(entries, cq);
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
struct Preparing {
    target: Target,
    operation: OperationId,
    record: Option<DetachedOp>,
    event: Option<crate::timers::PreparedEvent>,
}

impl Preparing {
    fn plain(
        target: Target,
        user_data: u64,
        ids: &operation_ids::OperationIds,
    ) -> Result<Self, SubmitError> {
        let operation = ids.allocate().ok_or(SubmitError::WouldBlock)?;
        let charge = budget::reserve(&target.records, target.platform)
            .map_err(|_| SubmitError::WouldBlock)?;
        let cq = target.cq;
        Ok(Self {
            target,
            operation,
            record: Some(DetachedOp {
                user_data,
                cq,
                cancel_pending: false,
                _timer_observer: None,
                timer_cancel: None,
                record_charge: Some(charge),
            }),
            event: None,
        })
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
        let charge =
            crate::timers::budget::reserve(&preparing.target.timers, preparing.target.platform)
                .map_err(|_| SubmitError::WouldBlock)?;
        let (event, cancel) =
            TimerEvent::charged(ExtDuration::from_millis(timeout_ms as u128), charge)
                .map_err(|_| SubmitError::WouldBlock)?;
        preparing.record.as_mut().unwrap().timer_cancel = Some(TimerCancellation(Some(cancel)));
        preparing.event = Some(allocate_event(event).map_err(|_| SubmitError::WouldBlock)?);
        let observer = allocate(DetachedTimerObserver {
            asid: preparing.target.asid,
            operation: preparing.operation,
        })
        .map_err(|_| SubmitError::WouldBlock)?;
        preparing
            .event
            .as_ref()
            .unwrap()
            .event()
            .register_observer(Arc::downgrade(&observer) as Weak<dyn Observer>);
        preparing.record.as_mut().unwrap()._timer_observer = Some(observer);
        Ok(preparing)
    }

    fn publish(&mut self) -> Result<OperationId, SubmitError> {
        let mut registry = COMPLETIONS.write();
        let entries =
            registry.get_mut(&self.target.asid).ok_or(SubmitError::UnknownAddressSpace)?;
        self.target.validate(entries)?;
        flush_cq_backlog(entries, self.target.cq);
        if entries.live >= entries.capacity {
            return Err(SubmitError::WouldBlock);
        }
        // Registry-map allocation is separate work. The complete payload
        // already exists; no timer/node/observer allocation remains here.
        assert!(
            entries
                .detached
                .insert(
                    self.operation,
                    self.record.take().expect("detached preparation published twice")
                )
                .is_none()
        );
        entries.live += 1;
        Ok(self.operation)
    }

    fn enqueue(mut self) {
        assert!(self.record.is_none(), "unpublished detached timer cannot enqueue");
        if let Some(event) = self.event.take() {
            event.enqueue();
        }
    }
}

pub(super) fn submit(
    asid: AddressSpaceId,
    cq: CqId,
    user_data: u64,
    ids: &operation_ids::OperationIds,
) -> Result<OperationId, SubmitError> {
    let mut preparing = Preparing::plain(Target::capture(asid, cq)?, user_data, ids)?;
    let operation = preparing.publish()?;
    preparing.enqueue();
    Ok(operation)
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
    let operation = preparing.publish()?;
    preparing.enqueue();
    Ok(operation)
}

mod tests;

pub(super) fn test_preparation() {
    tests::run();
}
