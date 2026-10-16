//! Prepare complete capability-backed payloads outside COMPLETIONS. Hidden
//! authority admission and joint publication revalidate the captured namespace.

use core::mem::ManuallyDrop;

use super::*;
use crate::capability::{
    ObjectKind,
    PreparedReservation,
    Publication,
    Reservation,
};

struct Target {
    asid: AddressSpaceId,
    root: Option<crate::memory::AddressSpaceHandle>,
    records: Arc<budget::DomainBudget>,
    timers: Arc<crate::timers::budget::DomainBudget>,
    watches: Arc<watch_budget::DomainBudget>,
    capacity: usize,
    platform: bool,
}

impl Target {
    fn capture(asid: AddressSpaceId) -> Result<Self, SubmitError> {
        let platform = crate::memory::budget::platform_identity(asid);
        // The refund owner outlives this local registry guard on every path.
        let mut releases = budget::Releases::default();
        let mut registry = COMPLETIONS.write();
        let entries = registry.get_mut(&asid).ok_or(SubmitError::UnknownAddressSpace)?;
        if entries.address_space.is_some_and(|root| !crate::memory::budget::accepting(root)) {
            return Err(SubmitError::UnknownAddressSpace);
        }
        flush_cq_backlog(entries, DEFAULT_CQ, &mut releases);
        if entries.live >= entries.capacity {
            return Err(SubmitError::WouldBlock);
        }
        Ok(Self {
            asid,
            root: entries.address_space,
            records: entries.record_budget.clone(),
            timers: entries.timer_budget.clone(),
            watches: entries.watch_budget.clone(),
            capacity: entries.capacity,
            platform: asid == crate::memory::KERNEL_ASID
                || platform.is_some_and(|root| Some(root) == entries.address_space),
        })
    }

    fn validate(&self, entries: &AsCompletions) -> Result<(), SubmitError> {
        if self.root != entries.address_space
            || !Arc::ptr_eq(&self.records, &entries.record_budget)
            || !Arc::ptr_eq(&self.timers, &entries.timer_budget)
            || !Arc::ptr_eq(&self.watches, &entries.watch_budget)
            || self.capacity != entries.capacity
            || self.root.is_some_and(|root| !crate::memory::budget::accepting(root))
        {
            return Err(SubmitError::UnknownAddressSpace);
        }
        Ok(())
    }
}

/// Explicit finish runs after local guards on every ordinary success/error.
/// Abandonment retains all fields, including hidden authority and original
/// charges; it never cancels, frees or enters another registry in Drop.
#[must_use]
struct Preparing(ManuallyDrop<Resources>);
struct Resources {
    target: Target,
    buffer: Option<Vec<u8>>,
    operation: Option<OperationId>,
    record_charge: Option<budget::Charge>,
    completion: Option<CompletionRef>,
    entry: Option<PreparedEntry<(CompletionCap, CapabilityRecord)>>,
    watch: Option<watch_budget::Charge>,
    authority: Option<PreparedReservation>,
    reservation: Option<Reservation>,
    event: Option<crate::timers::PreparedEvent>,
    cancel: Option<TimerCancellation>,
    published: bool,
}

impl Preparing {
    fn new(
        target: Target,
        buffer: Option<Vec<u8>>,
        watch: bool,
        ids: &operation_ids::OperationIds,
    ) -> Result<Self, SubmitError> {
        Self::new_with(
            target,
            buffer,
            watch,
            ids,
            Self::record,
            |asid, root| PreparedReservation::try_new(asid, ObjectKind::Completion, root),
            PreparedEntry::try_new,
        )
    }

    fn new_with(
        target: Target,
        buffer: Option<Vec<u8>>,
        watch: bool,
        ids: &operation_ids::OperationIds,
        allocate_record: impl FnOnce(&mut Self) -> Result<(), SubmitError>,
        allocate_authority: impl FnOnce(
            AddressSpaceId,
            Option<crate::memory::AddressSpaceHandle>,
        ) -> Result<
            PreparedReservation,
            crate::capability::AllocationError,
        >,
        allocate_node: impl FnOnce() -> Result<
            PreparedEntry<(CompletionCap, CapabilityRecord)>,
            core::alloc::AllocError,
        >,
    ) -> Result<Self, SubmitError> {
        let mut preparing = Self::capture(target, buffer);
        let result = (|| {
            preparing.admit(watch, ids)?;
            // Factories borrow this complete transaction. Rejection cannot
            // destroy an owning buffer/charge argument inside the callback.
            allocate_record(&mut preparing)?;
            let resources = &mut *preparing.0;
            assert!(resources.completion.is_some(), "record factory returned no record");
            assert!(resources.buffer.is_none() && resources.record_charge.is_none());
            resources.entry = Some(allocate_node().map_err(|_| SubmitError::WouldBlock)?);
            resources.authority = Some(
                allocate_authority(resources.target.asid, resources.target.root)
                    .map_err(|_| SubmitError::WouldBlock)?,
            );
            preparing.reserve()
        })();
        if let Err(error) = result {
            preparing.finish();
            return Err(error);
        }
        Ok(preparing)
    }

    fn capture(target: Target, buffer: Option<Vec<u8>>) -> Self {
        Self(ManuallyDrop::new(Resources {
            target,
            buffer,
            operation: None,
            record_charge: None,
            completion: None,
            entry: None,
            watch: None,
            authority: None,
            reservation: None,
            event: None,
            cancel: None,
            published: false,
        }))
    }

    fn admit(&mut self, watch: bool, ids: &operation_ids::OperationIds) -> Result<(), SubmitError> {
        let resources = &mut *self.0;
        assert!(resources.operation.is_none() && resources.record_charge.is_none());
        resources.operation = Some(ids.allocate().ok_or(SubmitError::WouldBlock)?);
        resources.record_charge = Some(
            budget::reserve(&resources.target.records, resources.target.platform)
                .map_err(|_| SubmitError::WouldBlock)?,
        );
        if watch {
            resources.watch = Some(
                watch_budget::reserve(&resources.target.watches, resources.target.platform)
                    .map_err(|_| SubmitError::WouldBlock)?,
            );
        }
        Ok(())
    }

    fn record(&mut self) -> Result<(), SubmitError> {
        let resources = &mut *self.0;
        assert!(resources.completion.is_none());
        let operation = resources.operation.expect("record construction precedes identity");
        let charge =
            resources.record_charge.take().expect("record construction precedes admission");
        // Transfer directly into the inner complete-record owner before its
        // first fallible phase. The outer watch and target remain here.
        resources.completion = Some(Completion::new(operation, resources.buffer.take(), charge)?);
        Ok(())
    }

    fn reserve(&mut self) -> Result<(), SubmitError> {
        // The refund owner outlives this local registry guard on every path.
        let mut releases = budget::Releases::default();
        let mut registry = COMPLETIONS.write();
        let resources = &mut *self.0;
        let entries =
            registry.get_mut(&resources.target.asid).ok_or(SubmitError::UnknownAddressSpace)?;
        resources.target.validate(entries)?;
        flush_cq_backlog(entries, DEFAULT_CQ, &mut releases);
        if entries.live >= entries.capacity {
            return Err(SubmitError::WouldBlock);
        }
        assert!(resources.reservation.is_none());
        resources.reservation = Some(
            resources
                .authority
                .as_mut()
                .unwrap()
                .reserve_captured()
                .map_err(|_| SubmitError::WouldBlock)?,
        );
        Ok(())
    }

    fn timer(
        &mut self,
        deadline: crate::timers::Timestamp,
        allocate_event: impl FnOnce(TimerEvent) -> Result<crate::timers::PreparedEvent, ()>,
        allocate_observer: impl FnOnce(
            CompletionTimerObserver,
        )
            -> Result<Arc<CompletionTimerObserver>, core::alloc::AllocError>,
    ) -> Result<(), SubmitError> {
        let resources = &mut *self.0;
        assert!(!resources.published && resources.event.is_none() && resources.cancel.is_none());
        let charge =
            crate::timers::budget::reserve(&resources.target.timers, resources.target.platform)
                .map_err(|_| SubmitError::WouldBlock)?;
        let (event, cancel) =
            TimerEvent::charged_at(deadline, charge).map_err(|_| SubmitError::WouldBlock)?;
        resources.cancel = Some(TimerCancellation(Some(cancel)));
        resources.event = Some(allocate_event(event).map_err(|_| SubmitError::WouldBlock)?);
        let completion = resources.completion.as_ref().unwrap();
        let observer = allocate_observer(CompletionTimerObserver {
            asid: resources.target.asid,
            cap: resources.reservation.as_ref().unwrap().identity(),
            result: OpResult::Ok(0),
            completion: Arc::downgrade(completion),
        })
        .map_err(|_| SubmitError::WouldBlock)?;
        resources
            .event
            .as_ref()
            .unwrap()
            .event()
            .register_observer(Arc::downgrade(&observer) as Weak<dyn Observer>);
        completion.set_timer_observer(observer, resources.cancel.take().unwrap());
        Ok(())
    }

    fn publish(&mut self) -> Result<CompletionCap, SubmitError> {
        // The refund owner outlives this local registry guard on every path.
        let mut releases = budget::Releases::default();
        let mut registry = COMPLETIONS.write();
        let resources = &mut *self.0;
        assert!(!resources.published, "completion preparation published twice");
        let entries =
            registry.get_mut(&resources.target.asid).ok_or(SubmitError::UnknownAddressSpace)?;
        resources.target.validate(entries)?;
        flush_cq_backlog(entries, DEFAULT_CQ, &mut releases);
        if entries.live >= entries.capacity {
            return Err(SubmitError::WouldBlock);
        }
        let reservation = resources.reservation.as_mut().unwrap();
        let cap = reservation.identity();
        assert!(!entries.table.contains_key(&cap), "completion capability replaced");
        crate::capability::publish_batch(&mut [Publication {
            destination: reservation,
            source: None,
        }])
        .map_err(|_| SubmitError::WouldBlock)?;
        entries.table.insert(
            resources.entry.take().expect("completion metadata published twice"),
            cap,
            CapabilityRecord::new(resources.completion.as_ref().unwrap().clone()),
        );
        entries.live += 1;
        resources.published = true;
        Ok(cap)
    }

    fn enqueue(&mut self) {
        assert!(self.0.published, "unpublished completion timer cannot enqueue");
        // Leave the published registry as the record owner before enqueue.
        // If namespace close won, the last record drop cancels this still
        // prepared node; its original event charge survives cancelled discard.
        drop(self.0.completion.take());
        if let Some(event) = self.0.event.take() {
            event.enqueue();
        }
    }

    fn finish(self) {
        self.finish_with(|| {});
    }

    fn finish_with(self, mut before_early_disposal: impl FnMut()) {
        let mut resources = ManuallyDrop::into_inner(self.0);
        // Unused payload storage leaves before its original charged record.
        drop(resources.entry.take());
        if let Some(authority) = resources.authority.take() {
            authority.finish();
        }
        // An active reservation revokes only its captured hidden entry. The
        // complete payload still belongs to resources until guard-free exit.
        drop(resources.reservation.take());
        if let Some(event) = resources.event.take() {
            event.cancel_unpublished();
        }
        // Early rejection has no record owner yet. Keep its original charge
        // until the submitted buffer and all unused metadata have gone.
        before_early_disposal();
        drop(resources.buffer.take());
        before_early_disposal();
        drop(resources.record_charge.take());
        drop(resources);
    }
}

pub(super) fn submit(
    asid: AddressSpaceId,
    buffer: Option<Vec<u8>>,
    watch: bool,
    ids: &operation_ids::OperationIds,
) -> Result<(CompletionCap, CompletionRef, Option<watch_budget::Charge>), SubmitError> {
    let mut preparing = Preparing::new(Target::capture(asid)?, buffer, watch, ids)?;
    let result = preparing.publish().map(|cap| {
        (cap, preparing.0.completion.as_ref().unwrap().clone(), preparing.0.watch.take())
    });
    preparing.finish();
    result
}

pub(super) fn submit_timer(
    asid: AddressSpaceId,
    timeout_ms: u64,
    ids: &operation_ids::OperationIds,
    allocate: impl FnOnce(
        CompletionTimerObserver,
    ) -> Result<Arc<CompletionTimerObserver>, core::alloc::AllocError>,
) -> Result<CompletionCap, SubmitError> {
    let deadline = crate::timers::deadline_after(ExtDuration::from_millis(timeout_ms as u128));
    let mut preparing = Preparing::new(Target::capture(asid)?, None, false, ids)?;
    let result = preparing
        .timer(deadline, crate::timers::PreparedEvent::new, allocate)
        .and_then(|()| preparing.publish());
    if result.is_ok() {
        preparing.enqueue();
    }
    preparing.finish();
    result
}

mod initial_tests;
mod metadata_tests;
mod tests;

pub(super) fn test_preparation() {
    tests::run();
    metadata_tests::run();
    initial_tests::run();
}
