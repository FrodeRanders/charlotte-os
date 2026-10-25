//! Captured CQ lifetime, readiness registration, checked waiting and detached
//! wake delivery. Queue state stays behind the existing completion registry;
//! source identity retains no ring backing or physical completion authority.
use super::*;

mod waiter_tests;

pub(in crate::completion) fn signal_cq_source(
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

pub(crate) fn cq_waiter_count(asid: AddressSpaceId, cq: CqId) -> usize {
    COMPLETIONS
        .read()
        .get(&asid)
        .and_then(|entries| entries.cqs.get(&cq))
        .map_or(0, |state| state.waiters.registered())
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
    /// Borrow the captured admitted source for existing completion-family
    /// fixtures. This adds no ring/root lease or result-observation authority.
    pub(in crate::completion) fn waiter_source(
        &self,
    ) -> &crate::klib::observer::registration::ListRef<crate::klib::observer::waiter_budget::Charge>
    {
        &self.identity
    }

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
                let result = self.publish_waiter(&mut prepared);
                prepared.finish();
                result
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

/// Kernel fixture: a detached wake batch retains its captured CQ across
/// numeric ASID/CQ reuse. Reentrant callbacks
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
