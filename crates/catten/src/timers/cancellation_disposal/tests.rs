//! Synchronous pre-AP handback controls with private original sponsors.

use alloc::{
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::{
    cpu::{
        isa::lp::ops::get_int_state,
        multiprocessor::interrupt_tracking::LocalInterruptMask,
    },
    klib::{
        observer::{
            CallOnNotify,
            Observable,
            Observer,
            WaitSponsor,
        },
        time::duration::ExtDuration,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
    timers::{
        TIMER_QUEUES,
        budget,
        cancel_event,
        prepare_watchdog,
        process_local_events,
    },
};

fn context() {
    assert!(TIMER_QUEUES.try_get_mut().is_ok());
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
}

fn complete_handbacks() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("cancelled event notified"));
    let mut handles = Vec::with_capacity(64);
    let mut tokens = Vec::with_capacity(64);
    let mut receipts = Vec::with_capacity(64);
    for _ in 0..64 {
        let (node, handle) = prepare_watchdog(ExtDuration::from_millis(60_000), &sponsor).unwrap();
        node.event().register_observer(Arc::downgrade(&observer));
        tokens.push(node.event().try_register_waiter(Arc::downgrade(&observer), &waits).unwrap());
        node.enqueue();
        handles.push(handle);
    }
    let weak = Arc::downgrade(&handles[0].state);
    let mask = LocalInterruptMask::new();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        // Each cancellation's own queue transaction is allowed to complete;
        // no node/source/reference destruction may occur before handback.
        for handle in handles.drain(..) {
            let cancelled = cancel_event(handle);
            assert!(cancelled.removed());
            receipts.push(cancelled);
        }
        assert_eq!(sponsor.used(), 64);
        assert_eq!(waits.used(), 64);
    }
    for receipt in receipts {
        assert!(receipt.finish_with(context));
    }
    assert_eq!(waits.used(), 0);
    assert_eq!(sponsor.used(), 1);
    assert!(weak.upgrade().is_none());
    assert_eq!(Arc::weak_count(&observer), 0);
    drop((weak, tokens));
    assert_eq!(sponsor.used(), 0);
    drop(mask);

    // A stale final reference must survive even a busy queue + heap hold.
    let (node, handle) = prepare_watchdog(ExtDuration::from_millis(60_000), &sponsor).unwrap();
    node.cancel_unpublished();
    let cancelled;
    let mask = LocalInterruptMask::new();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        cancelled = cancel_event(handle);
        assert!(!cancelled.removed());
        assert_eq!(sponsor.used(), 1);
    }
    assert!(!cancelled.finish_with(context));
    assert_eq!(sponsor.used(), 0);
    drop(mask);

    // Remote cancellation retains its reference until explicit finish; flag
    // delivery and original-owner queue reclamation remain separate.
    let (node, handle) = prepare_watchdog(ExtDuration::from_millis(60_000), &sponsor).unwrap();
    handle
        .state
        .owner_lp
        .store(crate::cpu::isa::lp::ops::get_lp_id().wrapping_add(1), Ordering::Release);
    let cancelled = cancel_event(handle);
    assert!(!cancelled.removed());
    assert!(cancelled.handle().state.cancelled.load(Ordering::Acquire));
    assert!(!cancelled.finish_with(context));
    assert_eq!(sponsor.used(), 1);
    node.cancel_unpublished();
    assert_eq!(sponsor.used(), 0);
}

// Both fallbacks deliberately retain their fields instead of implicit cleanup.
#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        count.fetch_add(1, Ordering::Relaxed);
    });
    let (mut node, handle) = prepare_watchdog(ExtDuration::from_millis(60_000), &sponsor).unwrap();
    node.event_mut().deadline = 0;
    node.event().register_observer(Arc::downgrade(&observer));
    let identity = Arc::downgrade(&handle.state);
    let mask = LocalInterruptMask::new();
    node.enqueue();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        drop(handle);
        assert_eq!(identity.strong_count(), 2);
        assert!(!identity.upgrade().unwrap().cancelled.load(Ordering::Acquire));
    }
    process_local_events();
    assert_eq!(hits.load(Ordering::Relaxed), 1, "handle abandonment fabricated cancellation");
    assert_eq!(identity.strong_count(), 1);
    assert_eq!(sponsor.used(), 1);
    drop(identity);

    let (node, handle) = prepare_watchdog(ExtDuration::from_millis(60_000), &sponsor).unwrap();
    node.event().register_observer(Arc::downgrade(&observer));
    let token = node.event().try_register_waiter(Arc::downgrade(&observer), &waits).unwrap();
    node.enqueue();
    let cancelled = cancel_event(handle);
    assert!(cancelled.removed());
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        drop(cancelled);
        assert_eq!(sponsor.used(), 2);
        assert_eq!(waits.used(), 1);
    }
    drop(token);
    assert_eq!(waits.used(), 0);
    assert_eq!(sponsor.used(), 2);
    process_local_events();
    assert_eq!(hits.load(Ordering::Relaxed), 1);
    drop(mask);
    crate::logln!(
        "[timer cancellation retention] guarded final-handle and complete removed-node handback \
         abandonment retained two original event charges and one source; no fabricated flag or \
         notification, independent waiter cancellation preserved"
    );
}

pub(super) fn run() {
    let baseline = budget::node_used();
    let irq = get_int_state();
    complete_handbacks();
    assert_eq!(budget::node_used(), baseline);
    guarded_abandonment();
    assert_eq!(budget::node_used(), (baseline.0 + 2, baseline.1 + 2));
    assert_eq!(get_int_state(), irq);
    crate::logln!(
        "[timer cancellation handback] 64 heap-held complete removals, post-queue node and \
         final-reference disposal, stale final reference under busy queue, remote flag-only \
         cancellation, weak-only original charges and IRQ policy passed"
    );
}
