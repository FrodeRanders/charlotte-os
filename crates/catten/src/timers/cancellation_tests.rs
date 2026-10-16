//! Real LP-queue cancellation with identical metadata and retained predecessor
//! state. Private aliases exercise repeated stale requests without raw handles.

use alloc::sync::Arc;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::{
    cpu::multiprocessor::interrupt_tracking::LocalInterruptMask,
    klib::observer::CallOnNotify,
};

fn prepare(
    sponsor: &budget::SchedulerSponsor,
    deadline: Timestamp,
) -> (PreparedEvent, TimerEventCancelHandle) {
    let (event, handle) = TimerEvent::charged_at(deadline, sponsor.reserve().unwrap()).unwrap();
    (PreparedEvent::new(event).unwrap(), handle)
}

fn alias(handle: &TimerEventCancelHandle) -> TimerEventCancelHandle {
    // Serialized kernel fixture boundary: clone the existing typed allocation
    // owner, never adopt a scalar handle or reconstruct an allocation address.
    TimerEventCancelHandle {
        state: handle.state.clone(),
    }
}

fn exact_live_removal() {
    let first = budget::SchedulerSponsor::new(false);
    let second = budget::SchedulerSponsor::new(false);
    let (a, a_handle) = prepare(&first, Timestamp::MAX);
    let (b, b_handle) = prepare(&second, Timestamp::MAX);
    assert!(!Arc::ptr_eq(&a_handle.state, &b_handle.state));
    a.enqueue();
    b.enqueue();
    assert_eq!(
        a_handle.state.owner_lp.load(Ordering::Acquire),
        b_handle.state.owner_lp.load(Ordering::Acquire)
    );
    assert!(cancel_event(a_handle));
    assert_eq!((first.used(), second.used()), (0, 1));
    assert!(!b_handle.state.cancelled.load(Ordering::Acquire));
    assert!(cancel_event(b_handle));
    assert_eq!((first.used(), second.used()), (0, 0));
}

fn stale_unpublished_and_weak_retention() {
    let predecessor = budget::SchedulerSponsor::new(false);
    let successor = budget::SchedulerSponsor::new(false);
    let (prepared, stale) = prepare(&predecessor, Timestamp::MAX);
    let weak = Arc::downgrade(&stale.state);
    prepared.cancel_unpublished();
    predecessor.retire();
    assert_eq!(predecessor.used(), 1);
    assert!(predecessor.reserve().is_err());
    for _ in 0..64 {
        let (fresh, handle) = prepare(&successor, Timestamp::MAX);
        assert!(!Arc::ptr_eq(&stale.state, &handle.state));
        fresh.enqueue();
        assert!(!cancel_event(alias(&stale)), "unpublished predecessor removed a successor");
        assert_eq!((predecessor.used(), successor.used()), (1, 1));
        assert!(!handle.state.cancelled.load(Ordering::Acquire));
        assert!(cancel_event(handle));
        assert_eq!((predecessor.used(), successor.used()), (1, 0));
    }
    drop(stale);
    assert!(weak.upgrade().is_none());
    assert_eq!(predecessor.used(), 1, "weak-only backing must keep its original charge");
    let (fresh, handle) = prepare(&successor, Timestamp::MAX);
    fresh.enqueue();
    drop(weak);
    assert_eq!((predecessor.used(), successor.used()), (0, 1));
    assert!(cancel_event(handle));
    assert_eq!(successor.used(), 0);
}

fn stale_fired_preserves_notification() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        count.fetch_add(1, Ordering::Relaxed);
    });
    let (old, stale) = prepare(&sponsor, 0);
    old.event().register_observer(Arc::downgrade(&observer));
    old.enqueue();
    process_local_events();
    assert_eq!(hits.load(Ordering::Relaxed), 1);
    assert_eq!(sponsor.used(), 1, "fired node leaves retained cancellation backing");

    let (fresh, handle) = prepare(&sponsor, 0);
    fresh.event().register_observer(Arc::downgrade(&observer));
    {
        // Storage already exists. Mask only publication/stale cancellation so
        // this expired successor cannot fire before the exact-removal check.
        let _mask = LocalInterruptMask::new();
        fresh.enqueue();
        assert!(!cancel_event(alias(&stale)), "fired predecessor removed a successor");
        assert_eq!(sponsor.used(), 2);
        assert!(!handle.state.cancelled.load(Ordering::Acquire));
    }
    process_local_events();
    assert_eq!(hits.load(Ordering::Relaxed), 2, "successor notification was lost");
    assert!(!cancel_event(handle));
    assert_eq!(sponsor.used(), 1);
    assert!(!cancel_event(stale));
    assert_eq!(sponsor.used(), 0);
}

fn busy_cancellation_removes_only_one() {
    let first = budget::SchedulerSponsor::new(false);
    let second = budget::SchedulerSponsor::new(false);
    let (a, a_handle) = prepare(&first, Timestamp::MAX);
    let (b, b_handle) = prepare(&second, Timestamp::MAX);
    let a_retry = alias(&a_handle);
    a.enqueue();
    b.enqueue();
    {
        let _mask = LocalInterruptMask::new();
        let queue = TIMER_QUEUES.try_get_mut().unwrap();
        assert!(!cancel_event(a_handle));
        assert!(!cancel_event(b_handle));
        assert_eq!((first.used(), second.used()), (1, 1));
        drop(queue);
        // Both flags/LPs/deadlines match. Eager removal must consume only a,
        // leaving b's actual node charged until owner reconciliation.
        assert!(cancel_event(a_retry));
        assert_eq!((first.used(), second.used()), (0, 1));
    }
    process_local_events();
    assert_eq!((first.used(), second.used()), (0, 0));
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let baseline = budget::node_used();
    exact_live_removal();
    stale_unpublished_and_weak_retention();
    stale_fired_preserves_notification();
    busy_cancellation_removes_only_one();
    assert_eq!(budget::node_used(), baseline);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[timer cancellation identity] exact live removal, 64 stale predecessor requests, \
         weak-only original charges, fired successor notification and busy-queue identity passed"
    );
}
