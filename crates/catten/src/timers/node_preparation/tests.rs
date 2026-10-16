//! Synchronous pre-AP private allocation-phase controls; no live sponsor swap.

use alloc::{
    sync::Arc,
    vec::Vec,
};

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
    timers::{
        TIMER_QUEUES,
        budget,
    },
};

fn context() {
    assert!(TIMER_QUEUES.try_get_mut().is_ok());
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
}

fn controlled_phases() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("private timer node notified"));
    let mut preparing = Vec::with_capacity(64);
    let mut completed = Vec::with_capacity(64);
    let mut handles = Vec::with_capacity(64);
    let mut tokens = Vec::with_capacity(64);
    for deadline in 0..64 {
        let (event, handle) = TimerEvent::charged_at(deadline, sponsor.reserve().unwrap()).unwrap();
        event.register_observer(Arc::downgrade(&observer));
        tokens.push(event.try_register_waiter(Arc::downgrade(&observer), &waits).unwrap());
        let mut owner = PreparingNode::new(event);
        owner.allocate(|| Box::try_new_uninit().map_err(|_| ())).unwrap();
        preparing.push(owner);
        handles.push(handle);
    }
    let weak = Arc::downgrade(&handles[0].state);
    let mask = LocalInterruptMask::new();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        for (deadline, owner) in preparing.drain(..).enumerate() {
            let node = owner.complete();
            assert_eq!(node.event().get_deadline(), deadline as u64);
            completed.push(node);
        }
        assert_eq!(sponsor.used(), 64);
        assert_eq!(waits.used(), 64);
        assert_eq!(Arc::weak_count(&observer), 128);
    }
    for node in completed {
        node.cancel_unpublished_with(|_| context());
    }
    assert_eq!(waits.used(), 0);
    assert_eq!(Arc::weak_count(&observer), 0);
    for handle in handles {
        handle.release();
    }
    assert!(weak.upgrade().is_none());
    assert_eq!(sponsor.used(), 1);
    drop((weak, tokens));
    assert_eq!(sponsor.used(), 0);
    drop(mask);

    // Rejection returns with the same event/source still in the preparation.
    let (event, handle) = TimerEvent::charged_at(123, sponsor.reserve().unwrap()).unwrap();
    event.register_observer(Arc::downgrade(&observer));
    let token = event.try_register_waiter(Arc::downgrade(&observer), &waits).unwrap();
    let mut owner = PreparingNode::new(event);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        assert!(owner.allocate(|| Err(())).is_err());
        assert_eq!(owner.0.event.as_ref().unwrap().get_deadline(), 123);
        assert!(owner.0.storage.is_none());
        assert_eq!(sponsor.used(), 1);
        assert_eq!(waits.used(), 1);
        assert_eq!(Arc::weak_count(&observer), 2);
    }
    owner.cancel_with(context);
    handle.release();
    drop(token);
    assert_eq!(sponsor.used(), 0);
    assert_eq!(waits.used(), 0);

    // The ordinary constructor disposes only after the allocator callback's
    // own guards have left. It never hands the callback an owning event.
    let (event, handle) = TimerEvent::charged_at(456, sponsor.reserve().unwrap()).unwrap();
    event.register_observer(Arc::downgrade(&observer));
    let token = event.try_register_waiter(Arc::downgrade(&observer), &waits).unwrap();
    assert!(
        PreparedEvent::new_with(event, || {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _queue = TIMER_QUEUES.try_get_mut().unwrap();
            assert_eq!(sponsor.used(), 1);
            assert_eq!(waits.used(), 1);
            Err(())
        })
        .is_err()
    );
    assert_eq!(waits.used(), 0);
    assert_eq!(sponsor.used(), 1);
    handle.release();
    drop(token);
    assert_eq!(sponsor.used(), 0);

    // Confirmed unused empty backing is explicitly freed before payload and
    // outside admission, without initializing a node merely to destroy it.
    let (event, handle) = TimerEvent::charged_at(789, sponsor.reserve().unwrap()).unwrap();
    let mut owner = PreparingNode::new(event);
    owner.allocate(|| Box::try_new_uninit().map_err(|_| ())).unwrap();
    owner.cancel_with(context);
    handle.release();
    assert_eq!(sponsor.used(), 0);
}

#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("abandoned node phase notified"));
    let mut tokens = Vec::with_capacity(2);
    for allocated in [false, true] {
        let (event, handle) = TimerEvent::charged_at(0, sponsor.reserve().unwrap()).unwrap();
        event.register_observer(Arc::downgrade(&observer));
        tokens.push(event.try_register_waiter(Arc::downgrade(&observer), &waits).unwrap());
        let identity = Arc::downgrade(&handle.state);
        handle.release();
        let mut owner = PreparingNode::new(event);
        if allocated {
            owner.allocate(|| Box::try_new_uninit().map_err(|_| ())).unwrap();
        }
        let mask = LocalInterruptMask::new();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _queue = TIMER_QUEUES.try_get_mut().unwrap();
            drop(owner);
            assert_eq!(identity.strong_count(), 1);
            assert_eq!(
                sponsor.used(),
                if allocated {
                    2
                } else {
                    1
                }
            );
            assert_eq!(
                waits.used(),
                if allocated {
                    2
                } else {
                    1
                }
            );
        }
        drop(mask);
        drop(identity);
    }
    drop(tokens);
    assert_eq!(waits.used(), 0);
    assert_eq!(sponsor.used(), 2);
    assert_eq!(Arc::weak_count(&observer), 2);
    crate::logln!(
        "[timer node preparation retention] guarded pre-allocation and empty-backing abandonment \
         retained two original event charges, callback/source fields and one uninitialized Box; \
         independent waiter cancellation preserved"
    );
}

pub(super) fn run() {
    let baseline = budget::node_used();
    let irq = get_int_state();
    controlled_phases();
    assert_eq!(budget::node_used(), baseline);
    guarded_abandonment();
    assert_eq!(budget::node_used(), (baseline.0 + 2, baseline.1 + 2));
    assert_eq!(get_int_state(), irq);
    crate::logln!(
        "[timer node preparation] 64 heap/queue-held allocation-free initializations, retained \
         allocation rejection, post-callback rollback, unused empty backing, weak-only original \
         charges and IRQ preservation passed"
    );
}
