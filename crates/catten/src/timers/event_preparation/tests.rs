//! Synchronous pre-AP construction phases, private sponsors captured first.

use alloc::{
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::Ordering;

use super::*;
use crate::{
    cpu::{
        isa::lp::ops::get_int_state,
        multiprocessor::interrupt_tracking::LocalInterruptMask,
    },
    klib::observer::{
        CallOnNotify,
        Observable,
        Observer,
        WaitSponsor,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
    timers::TIMER_QUEUES,
};

fn context() {
    assert!(TIMER_QUEUES.try_get_mut().is_ok());
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
}

fn completed_events() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("unpublished event notified"));
    let mut owners = Vec::with_capacity(64);
    let mut events = Vec::with_capacity(64);
    let mut tokens = Vec::with_capacity(64);
    for deadline in 0..64 {
        let event = TimerEvent::from(deadline);
        event.register_observer(Arc::downgrade(&observer));
        tokens.push(event.try_register_waiter(Arc::downgrade(&observer), &waits).unwrap());
        let mut owner = PreparingEvent::new(event);
        owner.reserve(&sponsor).unwrap();
        owner.admit().unwrap();
        owner.cancellation().unwrap();
        owners.push(owner);
    }
    let weak = Arc::downgrade(owners[0].event().cancellation.as_ref().unwrap());
    let mask = LocalInterruptMask::new();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        for (deadline, owner) in owners.drain(..).enumerate() {
            let event = owner.complete();
            assert_eq!(event.get_deadline(), deadline as u64);
            assert!(!event.cancellation.as_ref().unwrap().cancelled.load(Ordering::Acquire));
            events.push(event);
        }
        assert_eq!(sponsor.used(), 64);
        assert_eq!(waits.used(), 64);
    }
    // Completed raw events retain their ordinary destructor policy. Qualify
    // this explicit fixture disposal after both holds, not every raw caller.
    context();
    drop(events);
    assert_eq!(waits.used(), 0);
    assert_eq!(Arc::weak_count(&observer), 0);
    assert!(weak.upgrade().is_none());
    assert_eq!(sponsor.used(), 1);
    sponsor.retire();
    let successor = budget::SchedulerSponsor::new(false);
    let fresh = successor.reserve().unwrap();
    drop((weak, tokens));
    assert_eq!(sponsor.used(), 0);
    assert_eq!(successor.used(), 1);
    drop(fresh);
    drop(mask);
}

fn rejected_preparation() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("rejected construction notified"));
    let mut full = Vec::with_capacity(budget::MAX_DOMAIN_TIMERS);
    for _ in 0..budget::MAX_DOMAIN_TIMERS {
        full.push(sponsor.reserve().unwrap());
    }
    let event = TimerEvent::from(123);
    event.register_observer(Arc::downgrade(&observer));
    let token = event.try_register_waiter(Arc::downgrade(&observer), &waits).unwrap();
    let mut owner = PreparingEvent::new(event);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        assert!(owner.reserve(&sponsor).is_err());
        assert_eq!(waits.used(), 1);
        assert_eq!(owner.event().get_deadline(), 123);
    }
    owner.cancel_with(context);
    assert_eq!(waits.used(), 0);
    drop(token);
    // The public sleep adapter takes the same explicit failure cleanup path.
    let event = TimerEvent::from(456);
    let token = event.try_register_waiter(Arc::downgrade(&observer), &waits).unwrap();
    assert!(event.prepare_sleep(&sponsor).is_err());
    assert_eq!(waits.used(), 0);
    assert_eq!(sponsor.used(), budget::MAX_DOMAIN_TIMERS);
    drop((token, full));
    assert_eq!(sponsor.used(), 0);

    let event = TimerEvent::from(789);
    event.register_observer(Arc::downgrade(&observer));
    let token = event.try_register_waiter(Arc::downgrade(&observer), &waits).unwrap();
    let mut owner = PreparingEvent::new(event);
    owner.reserve(&sponsor).unwrap();
    owner.admit().unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        assert!(owner.cancellation_with(|_| Err(())).is_err());
        assert!(owner.event().cancellation.is_none());
        assert_eq!(owner.event().get_deadline(), 789);
        assert_eq!(sponsor.used(), 1);
        assert_eq!(waits.used(), 1);
    }
    owner.cancel_with(context);
    drop(token);
    assert_eq!(waits.used(), 0);
    assert_eq!(sponsor.used(), 0);
}

#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("abandoned event construction notified"));
    let mut tokens = Vec::with_capacity(4);
    for phase in 0..4 {
        let event = TimerEvent::from(0);
        event.register_observer(Arc::downgrade(&observer));
        tokens.push(event.try_register_waiter(Arc::downgrade(&observer), &waits).unwrap());
        let mut owner = PreparingEvent::new(event);
        if phase >= 1 {
            owner.reserve(&sponsor).unwrap();
        }
        if phase >= 2 {
            owner.admit().unwrap();
        }
        if phase >= 3 {
            owner.cancellation().unwrap();
        }
        let mask = LocalInterruptMask::new();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _queue = TIMER_QUEUES.try_get_mut().unwrap();
            drop(owner);
            assert_eq!(sponsor.used(), phase);
            assert_eq!(waits.used(), phase + 1);
        }
        drop(mask);
    }
    drop(tokens);
    assert_eq!(sponsor.used(), 3);
    assert_eq!(waits.used(), 0);
    assert_eq!(Arc::weak_count(&observer), 4);
    crate::logln!(
        "[timer event construction retention] guarded source-only, reservation, holder and \
         cancellation phases retained three original event charges and four source/callback \
         fields; independent waiter cancellation preserved"
    );
}

pub(super) fn run() {
    let baseline = budget::node_used();
    let irq = get_int_state();
    completed_events();
    rejected_preparation();
    assert_eq!(budget::node_used(), baseline);
    guarded_abandonment();
    assert_eq!(budget::node_used(), (baseline.0 + 3, baseline.1 + 3));
    assert_eq!(get_int_state(), irq);
    crate::logln!(
        "[timer event construction] 64 guarded allocation-free completions, original \
         source/deadline/admission retention, full sponsor and cancellation rejection, public \
         sleep cleanup, weak-only original charges and IRQ preservation passed"
    );
}
