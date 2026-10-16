//! Synchronous pre-AP queue ownership probes. Heap holds are private fixtures,
//! never a license to call ordinary post-queue finish beneath an outer guard.

use alloc::vec::Vec;

use super::*;
use crate::{
    cpu::multiprocessor::interrupt_tracking::LocalInterruptMask,
    klib::observer::CallOnNotify,
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn prepared(
    sponsor: &budget::SchedulerSponsor,
    deadline: Timestamp,
) -> (PreparedEvent, TimerEventCancelHandle) {
    let (event, handle) = TimerEvent::charged_at(deadline, sponsor.reserve().unwrap()).unwrap();
    (PreparedEvent::new(event).unwrap(), handle)
}

fn check_disposal(_: &TimerEvent) {
    // Called immediately before actual event/node destruction by the same
    // explicit finish implementation. Neither local guard may still be held.
    assert!(TIMER_QUEUES.try_get_mut().is_ok());
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
}

fn exact_removals() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("cancelled timer notified"));
    let mut handles = Vec::with_capacity(64);
    let mut tokens = Vec::with_capacity(64);
    let mut retired = Vec::with_capacity(64);
    for _ in 0..64 {
        let (next, handle) = prepared(&sponsor, Timestamp::MAX);
        next.event().register_observer(Arc::downgrade(&observer));
        tokens.push(next.event().try_register_waiter(Arc::downgrade(&observer), &waits).unwrap());
        next.enqueue();
        handles.push(handle);
    }
    let weak = Arc::downgrade(&handles[0].state);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let mut queue = TIMER_QUEUES.try_get_mut().unwrap();
        for handle in &handles {
            let (removed, events) = queue.remove_cancellable_event(&handle.state);
            assert!(removed);
            retired.push(events);
        }
        assert_eq!(sponsor.used(), 64);
        assert_eq!(waits.used(), 64);
    }
    for events in retired {
        events.finish_with(check_disposal);
    }
    assert_eq!(waits.used(), 0);
    assert_eq!(sponsor.used(), 64, "cancellation owners retain original backing admission");
    for handle in handles {
        handle.release();
    }
    drop(tokens);
    assert!(weak.upgrade().is_none());
    assert_eq!(sponsor.used(), 1);
    sponsor.retire();
    assert!(sponsor.reserve().is_err());
    let successor = budget::SchedulerSponsor::new(false);
    let fresh = successor.reserve().unwrap();
    drop(weak);
    assert_eq!(sponsor.used(), 0);
    assert_eq!(successor.used(), 1);
    drop(fresh);
}

fn purge_and_rejected_enqueue() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("purged/rejected timer notified"));
    let (first, a) = prepared(&sponsor, Timestamp::MAX);
    let (second, b) = prepared(&sponsor, Timestamp::MAX);
    let a_token = first.event().try_register_waiter(Arc::downgrade(&observer), &waits).unwrap();
    let b_token = second.event().try_register_waiter(Arc::downgrade(&observer), &waits).unwrap();
    first.enqueue();
    let (a_cancelled, b_cancelled);
    {
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        a_cancelled = cancel_event(a);
        b_cancelled = cancel_event(b);
        assert!(!a_cancelled.removed());
        assert!(!b_cancelled.removed());
    }
    assert!(!a_cancelled.finish());
    assert!(!b_cancelled.finish());
    let retired;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let mut queue = TIMER_QUEUES.try_get_mut().unwrap();
        retired = queue.add_prepared(second.into_node());
        assert_eq!(sponsor.used(), 2);
        assert_eq!(waits.used(), 2);
    }
    retired.finish_with(check_disposal);
    assert_eq!(sponsor.used(), 0);
    assert_eq!(waits.used(), 0);
    drop((a_token, b_token));
}

fn expiry_and_keyed_events() {
    // Private queue writes the local comparator only under this fixture's
    // original mask. Restore the real queue's comparator before leaving run.
    let sponsor = budget::SchedulerSponsor::new(false);
    let hits = Arc::new(core::sync::atomic::AtomicUsize::new(0));
    let count = hits.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        count.fetch_add(1, Ordering::Relaxed);
    });
    let mut private = TimerQueue::default();
    let (due, a) = prepared(&sponsor, 0);
    let (future, b) = prepared(&sponsor, Timestamp::MAX);
    due.event().register_observer(Arc::downgrade(&observer));
    private.add_prepared(due.into_node()).finish();
    private.add_prepared(future.into_node()).finish();
    let mut quantum =
        TimerEvent::keyed(ExtDuration::from_millis(1), TimerEventKey::SchedulerQuantum);
    quantum.deadline = 0;
    quantum.register_observer(Arc::downgrade(&observer));
    private.ensure_event(quantum).finish();
    let duplicate = TimerEvent::keyed(ExtDuration::from_millis(1), TimerEventKey::SchedulerQuantum);
    let rejected;
    let expired;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        rejected = private.ensure_event(duplicate);
        assert_eq!(rejected.front().unwrap().key, Some(TimerEventKey::SchedulerQuantum));
        assert_eq!(private.events.front().unwrap().deadline, 0);
        expired = private.prepare_delivery().unwrap();
        assert_eq!(private.get_next_deadline(), Some(Timestamp::MAX));
        assert_eq!(hits.load(Ordering::Relaxed), 0);
        assert_eq!(sponsor.used(), 2);
    }
    rejected.finish_with(check_disposal);
    expired.deliver_with(|| private.complete_delivery());
    assert_eq!(hits.load(Ordering::Relaxed), 2);
    assert_eq!(sponsor.used(), 2);
    a.release();
    assert_eq!(sponsor.used(), 1);
    let remaining;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        let (removed, retired) = private.remove_cancellable_event(&b.state);
        assert!(removed);
        remaining = retired;
        assert_eq!(sponsor.used(), 1);
    }
    remaining.finish_with(check_disposal);
    b.release();
    assert_eq!(sponsor.used(), 0);
    private
        .ensure_event(TimerEvent::keyed(
            ExtDuration::from_millis(1),
            TimerEventKey::SchedulerQuantum,
        ))
        .finish();
    let retired;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        retired = private.remove_event(TimerEventKey::SchedulerQuantum);
        assert!(private.events.front().is_none());
    }
    retired.finish_with(check_disposal);
}

fn guarded_abandonment() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("abandoned timer notified"));
    let (next, handle) = prepared(&sponsor, Timestamp::MAX);
    let weak = Arc::downgrade(&handle.state);
    let token = next.event().try_register_waiter(Arc::downgrade(&observer), &waits).unwrap();
    next.enqueue();
    let retired;
    {
        let mut queue = TIMER_QUEUES.try_get_mut().unwrap();
        let (removed, events) = queue.remove_cancellable_event(&handle.state);
        assert!(removed);
        retired = events;
    }
    handle.release();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        drop(retired);
        assert_eq!(sponsor.used(), 1);
        assert_eq!(waits.used(), 1);
        assert!(weak.upgrade().is_some());
    }
    // Independent token cancellation releases only its own entry, not the
    // retained event/node/source or original event reservation.
    drop(token);
    assert_eq!(waits.used(), 0);
    assert_eq!(sponsor.used(), 1);
    drop(weak);
    assert_eq!(sponsor.used(), 1);
    crate::logln!(
        "[timer disposal retention] guarded complete receipt abandonment retained one original \
         event/node/cancellation charge and source; independent waiter cancellation preserved"
    );
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let baseline = budget::node_used();
    let mask = LocalInterruptMask::new();
    exact_removals();
    purge_and_rejected_enqueue();
    expiry_and_keyed_events();
    assert_eq!(budget::node_used(), baseline);
    guarded_abandonment();
    assert_eq!(budget::node_used(), (baseline.0 + 1, baseline.1 + 1));
    process_local_events();
    drop(mask);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[timer queue disposal] 64 heap-held exact removals, purge/rejected enqueue, expiry and \
         keyed handback, post-queue guard checks, weak-only original-account retention and IRQ \
         preservation passed"
    );
}
