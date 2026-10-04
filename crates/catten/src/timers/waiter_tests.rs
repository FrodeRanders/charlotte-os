//! Kernel fixtures for timer observer ownership, not timer-queue capacity.

use alloc::{
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::TimerEvent;
use crate::{
    cpu::scheduler::{
        SLEEP_WAIT_ADMISSION_FALLBACKS,
        block_until,
        monotonic_millis,
        sleep_millis,
        sleep_with_event,
        system_scheduler::{
            SYSTEM_SCHEDULER,
            get_thread_id,
        },
        threads::{
            MASTER_THREAD_TABLE,
            ThreadState,
        },
    },
    klib::{
        observer::{
            CallOnNotify,
            Observable,
            Observer,
            WaitSponsor,
            registration::RegistrationError,
            waiter_budget,
        },
        time::duration::ExtDuration,
    },
};

pub(crate) fn test_admission() {
    let baseline = waiter_budget::node_used();
    let sponsor = WaitSponsor::new(false);
    let event = Arc::new(TimerEvent::from(ExtDuration::from_millis(60_000)));
    let hits = Arc::new(AtomicUsize::new(0));
    let target = event.clone();
    let count = hits.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        assert_eq!(target.waiters.registered(), 0);
        assert!(target.callback.lock().is_none());
        count.fetch_add(1, Ordering::Relaxed);
    });
    event.register_observer(Arc::downgrade(&observer));
    let mut tokens = Vec::new();
    for _ in 0..64 {
        tokens.push(event.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    assert!(matches!(
        event.try_register_waiter(Arc::downgrade(&observer), &sponsor),
        Err(RegistrationError::ResourceLimit)
    ));
    assert_eq!(sponsor.used(), 64);
    drop(tokens.pop());
    assert_eq!(sponsor.used(), 63);
    tokens.push(event.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
    event.signal();
    assert_eq!(hits.load(Ordering::Relaxed), 65);
    assert_eq!(sponsor.used(), 0);
    event.signal();
    assert_eq!(hits.load(Ordering::Relaxed), 65);
    drop(tokens);
    for _ in 0..512 {
        drop(event.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
        assert_eq!(event.waiters.registered(), 0);
    }
    let expired: Arc<dyn Observer> = CallOnNotify::new(|| panic!("expired timer callback fired"));
    event.register_observer(Arc::downgrade(&expired));
    drop(expired);
    let token = event.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap();
    event.signal();
    assert_eq!(hits.load(Ordering::Relaxed), 66);
    drop(token);

    let (cancelled, handle) = TimerEvent::cancellable(ExtDuration::from_millis(60_000));
    cancelled.register_observer(Arc::downgrade(&observer));
    let token = cancelled.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap();
    handle.cancelled.store(true, Ordering::Release);
    cancelled.signal();
    assert_eq!(hits.load(Ordering::Relaxed), 66);
    assert_eq!(sponsor.used(), 1); // Suppression alone does not free queue storage.
    drop(cancelled);
    assert_eq!(sponsor.used(), 0);
    drop((token, handle));

    let discarded = TimerEvent::from(ExtDuration::from_millis(60_000));
    let token = discarded.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap();
    drop(discarded);
    assert_eq!(sponsor.used(), 0);
    drop(token);
    sponsor.retire();
    assert!(matches!(
        event.try_register_waiter(Arc::downgrade(&observer), &sponsor),
        Err(RegistrationError::Closed)
    ));
    drop((observer, event));
    assert_eq!(waiter_budget::node_used(), baseline);
    crate::logln!(
        "[timer waiters] SUCCESS: bounds/rollback, cancellation, callback reentrancy, expired \
         callback and source destruction"
    );
}

pub(crate) fn test_scheduled_cleanup() {
    // Successful sleeps exercise timer delivery and the owning scheduler token.
    for _ in 0..64 {
        sleep_millis(1);
    }
    // Competing watchdogs cancel an unqueued timer source without retaining entries.
    let event = TimerEvent::from(ExtDuration::from_millis(60_000));
    for _ in 0..64 {
        assert!(!block_until(&event, 1, || false));
        assert_eq!(event.waiters.registered(), 0);
    }
    let tid = get_thread_id().unwrap();
    let constraints = MASTER_THREAD_TABLE.read().get(tid).unwrap().migration_constraints;
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| {});
    let mut tokens = Vec::new();
    for _ in 0..64 {
        tokens.push(event.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    assert!(matches!(
        SYSTEM_SCHEDULER.read().block_thread(tid, &event),
        Err(crate::cpu::scheduler::system_scheduler::Error::WaitRegistrationFailed)
    ));
    {
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(tid).unwrap();
        assert!(matches!(thread.state, ThreadState::Running(_)));
        assert_eq!(thread.migration_constraints, constraints);
    }
    let retries = SLEEP_WAIT_ADMISSION_FALLBACKS.load(Ordering::Relaxed);
    let started = monotonic_millis();
    sleep_with_event(ExtDuration::from_millis(2), event);
    assert!(monotonic_millis().saturating_sub(started) >= 2);
    assert!(SLEEP_WAIT_ADMISSION_FALLBACKS.load(Ordering::Relaxed) > retries);
    assert_eq!(sponsor.used(), 0); // Rejected event destroyed with retained tokens.
    drop(tokens);
    crate::logln!(
        "[timer waiters] SUCCESS: sleep delivery, timed cleanup, non-mutating rejection and \
         runnable sleep fallback"
    );
}
