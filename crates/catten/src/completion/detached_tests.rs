//! Real detached timer owners, CQ publication and local disposal boundaries.
//! Private fixtures retain their original accounts, not physical root proofs.

use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::{
    klib::observer::{
        CallOnNotify,
        WaitSponsor,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

const CLIENT: usize = 0x5e34;

fn heap_held_splice() {
    for mode in 0..3 {
        open_address_space_with_cq(CLIENT, 4, 2).unwrap();
        let records = record_admission(CLIENT).unwrap();
        if mode == 1 {
            let filler = submit_detached(CLIENT, DEFAULT_CQ, OpCode::Nop, 11).unwrap();
            complete_detached(CLIENT, filler, OpResult::Ok(11)).unwrap();
        }
        let operation = submit_detached_timer(CLIENT, DEFAULT_CQ, u64::MAX, 12).unwrap();
        let observer = {
            let registry = COMPLETIONS.read();
            Arc::downgrade(registry[&CLIENT].detached[&operation]._timer_observer.as_ref().unwrap())
        };
        if mode == 2 {
            let queue = COMPLETIONS.write().get_mut(&CLIENT).unwrap().cqs.take(&DEFAULT_CQ);
            if let Some(queue) = queue {
                RetiredCq::from_node(queue).finish();
            }
        }
        let retired;
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let mut registry = COMPLETIONS.write();
            let entries = registry.get_mut(&CLIENT).unwrap();
            let record = entries.detached.take(&operation).unwrap();
            retired = post_detached(CLIENT, operation, OpResult::Ok(12), entries, record);
            assert_eq!(entries.live, usize::from(mode == 1));
            assert_eq!(entries.timer_budget.used(), 1);
            assert_eq!(records.used(), 1);
            assert!(observer.upgrade().is_some());
            assert_eq!(retired.0.record.is_some(), mode != 1);
            assert_eq!(retired.0.source.is_some(), mode != 2);
        }
        retired.finish();
        assert!(observer.upgrade().is_none());
        assert_eq!(timer_events_used(CLIENT), 0);
        assert_eq!(records.used(), usize::from(mode == 1));
        if mode != 2 {
            let mut registry = COMPLETIONS.write();
            let queue = registry.get_mut(&CLIENT).unwrap().cqs.get_mut(&DEFAULT_CQ).unwrap();
            // SAFETY: this private heap ring has exactly this fixture consumer.
            let first = unsafe { (&mut *queue.ring).read().unwrap() };
            assert_eq!(
                first.cookie,
                if mode == 1 {
                    11
                } else {
                    12
                }
            );
        }
        if mode == 1 {
            assert_eq!(cq_pending(CLIENT, DEFAULT_CQ), 1);
            assert_eq!(records.used(), 0);
            let mut registry = COMPLETIONS.write();
            let queue = registry.get_mut(&CLIENT).unwrap().cqs.get_mut(&DEFAULT_CQ).unwrap();
            let entry = unsafe { (&mut *queue.ring).read().unwrap() };
            assert_eq!(entry.cookie, 12);
            assert_eq!(cq::fields_to_op_result(entry.status, entry.result), OpResult::Ok(12));
        }
        assert_eq!(
            complete_detached(CLIENT, operation, OpResult::Err(99)),
            Err(CapError::UnknownCap)
        );
        close_address_space(CLIENT);
        assert_eq!(records.used(), 0);
    }
}

fn public_producers() {
    // Keep the zero-deadline producer from firing before its CQ interest is
    // installed. Explicit processing below exercises its real queue callback.
    let _setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
    for mode in 0..3 {
        open_address_space_with_cq(CLIENT, 4, 4).unwrap();
        let records = record_admission(CLIENT).unwrap();
        let operation = submit_detached_timer(
            CLIENT,
            DEFAULT_CQ,
            if mode == 2 {
                0
            } else {
                u64::MAX
            },
            21,
        )
        .unwrap();
        let producer = {
            let registry = COMPLETIONS.read();
            Arc::downgrade(registry[&CLIENT].detached[&operation]._timer_observer.as_ref().unwrap())
        };
        let observed = producer.clone();
        let account = records.clone();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let callback: Arc<dyn Observer> = CallOnNotify::new(move || {
            let mut registry = COMPLETIONS.write();
            let entries = registry.get_mut(&CLIENT).unwrap();
            assert!(!entries.detached.contains_key(&operation));
            assert_eq!(entries.live, 0);
            assert_eq!(account.used(), 0);
            // A firing callback still owns its queue node and producer Arc.
            // Disposal must not refund that live event or fabricate quiescence.
            assert_eq!(entries.timer_budget.used(), usize::from(mode == 2));
            assert_eq!(observed.upgrade().is_some(), mode == 2);
            let queue = entries.cqs.get_mut(&DEFAULT_CQ).unwrap();
            let entry = unsafe { (&mut *queue.ring).read().unwrap() };
            assert_eq!(entry.operation, operation);
            assert_eq!(entry.cookie, 21);
            assert_eq!(
                cq::fields_to_op_result(entry.status, entry.result),
                if mode == 1 {
                    OpResult::Cancelled
                } else {
                    OpResult::Ok(0)
                }
            );
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let sponsor = WaitSponsor::new(false);
        let source = capture_cq_interest(CLIENT, DEFAULT_CQ).unwrap();
        let token = sponsor.register(&source.identity, Arc::downgrade(&callback)).unwrap();
        match mode {
            0 => complete_detached(CLIENT, operation, OpResult::Ok(0)).unwrap(),
            1 => assert_eq!(cancel_detached(CLIENT, operation), Ok(CancelState::CancelRequested)),
            _ => crate::timers::process_local_events(),
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(sponsor.used(), 0);
        assert_eq!(records.used(), 0);
        assert_eq!(timer_events_used(CLIENT), 0);
        assert!(producer.upgrade().is_none());
        assert_eq!(
            complete_detached(CLIENT, operation, OpResult::Err(99)),
            Err(CapError::UnknownCap)
        );
        assert_eq!(cancel_detached(CLIENT, operation), Err(CapError::UnknownCap));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        drop(token);
        close_address_space(CLIENT);
    }
}

fn source_replacement() {
    open_address_space_with_cq(CLIENT, 4, 4).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let operation = submit_detached_timer(CLIENT, DEFAULT_CQ, u64::MAX, 31).unwrap();
    let sponsor = WaitSponsor::new(false);
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let callback: Arc<dyn Observer> = CallOnNotify::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let original = capture_cq_interest(CLIENT, DEFAULT_CQ).unwrap();
    let old_token = sponsor.register(&original.identity, Arc::downgrade(&callback)).unwrap();
    // Force replacement precisely between real serialized publication and
    // producer disposal/notification, rather than relying on a timing race.
    let retired = retire_detached(CLIENT, operation, OpResult::Ok(0)).unwrap();
    assert_eq!(records.used(), 1);
    assert_eq!(timer_events_used(CLIENT), 1);
    open_cq(CLIENT, DEFAULT_CQ, 4).unwrap();
    assert_eq!(sponsor.used(), 0);
    let successor = capture_cq_interest(CLIENT, DEFAULT_CQ).unwrap();
    let new_token = sponsor.register(&successor.identity, Arc::downgrade(&callback)).unwrap();
    retired.finish();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(sponsor.used(), 1);
    assert_eq!(records.used(), 0);
    assert_eq!(timer_events_used(CLIENT), 0);
    assert_eq!(cq_pending(CLIENT, DEFAULT_CQ), 0);
    assert_eq!(complete_detached(CLIENT, operation, OpResult::Ok(99)), Err(CapError::UnknownCap));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop((old_token, new_token));
    assert_eq!(sponsor.used(), 0);
    close_address_space(CLIENT);
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    heap_held_splice();
    public_producers();
    source_replacement();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[detached timer disposal] heap-held admitted-node removal and direct/backlog/missing-CQ \
         splice, post-registry cancellation and charges, manual/cancel/firing producers, \
         duplicate rejection and exact successor-source fencing passed"
    );
}
