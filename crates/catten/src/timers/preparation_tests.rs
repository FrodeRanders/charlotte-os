//! Completed private-node ownership before publication; synchronous pre-AP
//! probes use original private sponsors, never replacement live accounts.

use alloc::vec::Vec;

use super::*;
use crate::{
    cpu::multiprocessor::interrupt_tracking::LocalInterruptMask,
    klib::observer::CallOnNotify,
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn disposal_context(_: &TimerEvent) {
    assert!(TIMER_QUEUES.try_get_mut().is_ok());
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
}

fn unused_nodes() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("unused preparation notified"));
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let mut nodes = Vec::with_capacity(64);
    let mut handles = Vec::with_capacity(64);
    let mut tokens = Vec::with_capacity(64);
    for _ in 0..64 {
        let (node, handle) = prepare_watchdog(ExtDuration::from_millis(60_000), &sponsor).unwrap();
        node.event().register_observer(Arc::downgrade(&observer));
        tokens.push(node.event().try_register_waiter(Arc::downgrade(&observer), &waits).unwrap());
        nodes.push(node);
        handles.push(handle);
    }
    let weak = Arc::downgrade(&handles[0].state);
    // Flag cancellation before publication is not a terminal queue result.
    assert!(!cancel_event(handles.pop().unwrap()));
    assert_eq!(sponsor.used(), 64);
    assert_eq!(waits.used(), 64);
    for node in nodes {
        node.cancel_unpublished_with(disposal_context);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    assert_eq!(waits.used(), 0);
    assert_eq!(sponsor.used(), 63);
    assert_eq!(Arc::weak_count(&observer), 0);
    drop((tokens, handles));
    assert!(weak.upgrade().is_none());
    assert_eq!(sponsor.used(), 1, "weak-only original cancellation backing stays charged");
    sponsor.retire();
    let successor = budget::SchedulerSponsor::new(false);
    let fresh = successor.reserve().unwrap();
    drop(weak);
    assert_eq!(sponsor.used(), 0);
    assert_eq!(successor.used(), 1);
    drop(fresh);

    // Noncancellable scheduler sleep preparation uses the same complete owner.
    // Explicit disposal preserves a caller's pre-existing mask too.
    let sleep = TimerEvent::from(0).prepare_sleep(&successor).unwrap();
    let mask = LocalInterruptMask::new();
    sleep.cancel_unpublished_with(disposal_context);
    assert!(!crate::cpu::isa::lp::ops::get_int_state());
    drop(mask);
    assert_eq!(successor.used(), 0);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
}

// Deliberately exercise the inert ManuallyDrop fallback under unrelated holds.
#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("abandoned preparation notified"));
    let (node, handle) = prepare_watchdog(ExtDuration::from_millis(60_000), &sponsor).unwrap();
    node.event().register_observer(Arc::downgrade(&observer));
    let token = node.event().try_register_waiter(Arc::downgrade(&observer), &waits).unwrap();
    let identity = Arc::downgrade(&handle.state);
    drop(handle);
    let mask = LocalInterruptMask::new();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        drop(node);
        assert_eq!(sponsor.used(), 1);
        assert_eq!(waits.used(), 1);
        assert_eq!(identity.strong_count(), 1);
        assert_eq!(Arc::weak_count(&observer), 2);
    }
    drop(mask);
    drop(token);
    assert_eq!(waits.used(), 0);
    assert_eq!(Arc::weak_count(&observer), 1);
    assert_eq!(identity.strong_count(), 1);
    drop(identity);
    assert_eq!(sponsor.used(), 1);
    crate::logln!(
        "[timer preparation retention] guarded unpublished owner abandonment retained one \
         original node/event/cancellation charge, callback and source; independent waiter \
         cancellation preserved"
    );
}

pub(super) fn run() {
    let baseline = budget::node_used();
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    unused_nodes();
    assert_eq!(budget::node_used(), baseline);
    guarded_abandonment();
    assert_eq!(budget::node_used(), (baseline.0 + 1, baseline.1 + 1));
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[timer preparation disposal] 64 complete unused watchdog nodes, source cleanup, \
         pre-publication cancellation, weak-only original charges, sleep preparation and caller \
         IRQ preservation passed"
    );
}
