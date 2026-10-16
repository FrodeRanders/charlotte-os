//! Pre-AP owner-LP delivery controls. All storage precedes selection/masking;
//! no live sponsor, user root or production delivery claim is replaced.

use alloc::vec::Vec;
use core::sync::atomic::AtomicUsize;

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

fn check_context(owner_lp: crate::cpu::isa::lp::LpId) {
    assert_eq!(crate::cpu::isa::lp::ops::get_lp_id(), owner_lp);
    assert!(!crate::cpu::isa::lp::ops::get_int_state());
    assert!(TIMER_QUEUES.try_get_mut().is_ok(), "notification retained timer queue borrow");
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
}

fn ordered_reentrant_prefix() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let owner_lp = crate::cpu::isa::lp::ops::get_lp_id();
    let order = Arc::new(Mutex::new(Vec::with_capacity(68)));
    let (fresh, fresh_handle) = prepared(&sponsor, 0);
    let fresh_order = order.clone();
    let fresh_observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        check_context(owner_lp);
        fresh_order.lock().push(10002);
    });
    fresh.event().register_observer(Arc::downgrade(&fresh_observer));
    let fresh = Arc::new(Mutex::new(Some(fresh)));
    let selected_cancel = Arc::new(Mutex::new(None::<TimerEventCancelHandle>));
    let queued_cancel = Arc::new(Mutex::new(None::<TimerEventCancelHandle>));
    let (future, future_handle) = prepared(&sponsor, Timestamp::MAX);
    *queued_cancel.lock() = Some(future_handle);
    let mut events = Vec::with_capacity(64);
    let mut handles = Vec::with_capacity(63);
    let mut observers = Vec::with_capacity(64);
    let mut tokens = Vec::with_capacity(2);
    let waiter_order = order.clone();
    let waiter: Arc<dyn Observer> = CallOnNotify::new(move || {
        check_context(owner_lp);
        waiter_order.lock().push(10001);
    });
    let suppressed: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("selected cancelled waiter notified"));
    for i in (1..=64).rev() {
        let (event, handle) = prepared(&sponsor, i);
        let log = order.clone();
        let fresh = fresh.clone();
        let selected_target = selected_cancel.clone();
        let queued_cancel = queued_cancel.clone();
        let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
            check_context(owner_lp);
            assert!(TIMER_QUEUES.try_get_mut().unwrap().delivering);
            log.lock().push(i);
            if i == 16 {
                fresh.lock().take().unwrap().enqueue();
                process_local_events();
                assert!(!log.lock().contains(&10002), "nested drain extended the active prefix");
            }
            if i == 32 {
                assert!(
                    !cancel_event(selected_target.lock().take().unwrap()),
                    "selected state was fabricated as a queued removal"
                );
                assert!(
                    cancel_event(queued_cancel.lock().take().unwrap()),
                    "reentrant cancellation failed to remove queued future event"
                );
            }
        });
        event.event().register_observer(Arc::downgrade(&observer));
        if i == 1 {
            tokens
                .push(event.event().try_register_waiter(Arc::downgrade(&waiter), &waits).unwrap());
        }
        if i == 48 {
            tokens.push(
                event.event().try_register_waiter(Arc::downgrade(&suppressed), &waits).unwrap(),
            );
            *selected_cancel.lock() = Some(handle);
        } else {
            handles.push(handle);
        }
        observers.push(observer);
        events.push(event);
    }
    let mask = LocalInterruptMask::new();
    future.enqueue();
    for event in events {
        event.enqueue();
    }
    let pending;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let mut queue = TIMER_QUEUES.try_get_mut().unwrap();
        pending = queue.prepare_delivery().unwrap();
        assert!(queue.delivering);
        assert!(queue.prepare_delivery().is_none());
        assert!(order.lock().is_empty());
        assert_eq!(waits.used(), 2);
        assert_eq!(sponsor.used(), 66);
    }
    pending.deliver();
    let expected: Vec<_> = (1..=64)
        .filter(|i| *i != 48)
        .flat_map(|i| {
            if i == 1 {
                [Some(i), Some(10001)]
            } else {
                [Some(i), None]
            }
        })
        .flatten()
        .collect();
    assert_eq!(&*order.lock(), &expected);
    assert!(!TIMER_QUEUES.try_get_mut().unwrap().delivering);
    assert_eq!(waits.used(), 0);
    assert_eq!(sponsor.used(), 64);
    process_local_events();
    assert_eq!(order.lock().last(), Some(&10002));
    assert_eq!(order.lock().len(), expected.len() + 1);
    drop((handles, fresh_handle, tokens));
    assert_eq!(sponsor.used(), 0);
    drop(mask);
}

struct DropProbe {
    holder: Arc<Mutex<Option<Arc<dyn Observer>>>>,
    notified: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
    owner_lp: crate::cpu::isa::lp::LpId,
}

impl Observer for DropProbe {
    fn notify(self: Arc<Self>) {
        check_context(self.owner_lp);
        self.notified.fetch_add(1, Ordering::Relaxed);
        drop(self.holder.lock().take());
    }
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        check_context(self.owner_lp);
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

fn observer_destruction() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let notified = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let owner_lp = crate::cpu::isa::lp::ops::get_lp_id();
    for nested_mask in [false, true] {
        let (event, handle) = prepared(&sponsor, 0);
        let mut holders = Vec::with_capacity(2);
        let mut token = None;
        for waiter in [false, true] {
            let holder = Arc::new(Mutex::new(None));
            let observer: Arc<dyn Observer> = Arc::new(DropProbe {
                holder: holder.clone(),
                notified: notified.clone(),
                dropped: dropped.clone(),
                owner_lp,
            });
            if waiter {
                token = Some(
                    event.event().try_register_waiter(Arc::downgrade(&observer), &waits).unwrap(),
                );
            } else {
                event.event().register_observer(Arc::downgrade(&observer));
            }
            *holder.lock() = Some(observer);
            holders.push(holder);
        }
        let mask = nested_mask.then(LocalInterruptMask::new);
        let irq = crate::cpu::isa::lp::ops::get_int_state();
        // Keep the expired publication stable until this wrapper captures it.
        let publication = LocalInterruptMask::new();
        event.enqueue();
        process_local_events();
        assert!(holders.iter().all(|holder| holder.lock().is_none()));
        assert_eq!(waits.used(), 0);
        assert_eq!(sponsor.used(), 1);
        drop((token, handle));
        assert_eq!(sponsor.used(), 0);
        drop(publication);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
        drop(mask);
    }
    assert_eq!(notified.load(Ordering::Relaxed), 4);
    assert_eq!(dropped.load(Ordering::Relaxed), 4);
}

fn quantum_tie_order() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let owner_lp = crate::cpu::isa::lp::ops::get_lp_id();
    let order = Arc::new(Mutex::new(Vec::with_capacity(4)));
    let mut private = TimerQueue::default();
    let mut handles = Vec::with_capacity(3);
    let mut observers = Vec::with_capacity(4);
    let mask = LocalInterruptMask::new();
    for (deadline, id) in [(1, 10), (1, 11), (2, 12)] {
        let (event, handle) = prepared(&sponsor, deadline);
        let log = order.clone();
        let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
            check_context(owner_lp);
            log.lock().push(id);
        });
        event.event().register_observer(Arc::downgrade(&observer));
        private.add_prepared(event.node).finish();
        handles.push(handle);
        observers.push(observer);
    }
    let log = order.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        check_context(owner_lp);
        log.lock().push(99);
    });
    let mut quantum =
        TimerEvent::keyed(ExtDuration::from_millis(1), TimerEventKey::SchedulerQuantum);
    quantum.deadline = 1;
    quantum.register_observer(Arc::downgrade(&observer));
    observers.push(observer);
    private.ensure_event(quantum).finish();
    let pending;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        pending = private.prepare_delivery().unwrap();
        assert!(order.lock().is_empty());
        assert_eq!(sponsor.used(), 3);
    }
    pending.deliver_with(|| private.complete_delivery());
    assert_eq!(&*order.lock(), &[99, 10, 11, 12]);
    assert!(!private.delivering);
    drop(handles);
    assert_eq!(sponsor.used(), 0);
    process_local_events();
    drop(mask);
}

// The inert owner is deliberately dropped with all of its original fields.
#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    let sponsor = budget::SchedulerSponsor::new(false);
    let waits = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("abandoned delivery notified"));
    let (due, a) = prepared(&sponsor, 0);
    let (cancelled, b) = prepared(&sponsor, Timestamp::MAX);
    let a_token = due.event().try_register_waiter(Arc::downgrade(&observer), &waits).unwrap();
    let b_token = cancelled.event().try_register_waiter(Arc::downgrade(&observer), &waits).unwrap();
    let mask = LocalInterruptMask::new();
    let mut private = TimerQueue::default();
    private.add_prepared(due.node).finish();
    private.add_prepared(cancelled.node).finish();
    b.state.cancelled.store(true, Ordering::Release);
    drop((a, b));
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _queue = TIMER_QUEUES.try_get_mut().unwrap();
        let pending = private.prepare_delivery().unwrap();
        assert_eq!(sponsor.used(), 2);
        assert_eq!(waits.used(), 2);
        drop(pending);
        assert!(private.delivering);
        assert!(private.prepare_delivery().is_none());
        assert!(private.events.front().is_none());
        assert_eq!(sponsor.used(), 2);
        assert_eq!(waits.used(), 2);
    }
    drop((a_token, b_token));
    assert_eq!(waits.used(), 0);
    assert_eq!(sponsor.used(), 2);
    // No real LP delivery claim was abandoned. Restore its comparator; the
    // private fixture stays fenced and supplies no recovery/retry selector.
    process_local_events();
    drop(mask);
    crate::logln!(
        "[timer delivery retention] guarded complete selected/purged owner abandonment retained \
         two original event charges and sources; private claim fenced and independent waiter \
         cancellation preserved"
    );
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let baseline = budget::node_used();
    ordered_reentrant_prefix();
    observer_destruction();
    quantum_tie_order();
    assert_eq!(budget::node_used(), baseline);
    guarded_abandonment();
    assert_eq!(budget::node_used(), (baseline.0 + 2, baseline.1 + 2));
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[timer callback delivery] 64 ordered heap-held selections, post-queue callback/waiter \
         and final observer destruction, selected/queued cancellation, nested drain fencing, \
         next-pass enqueue, original charges and IRQ policy passed"
    );
}
