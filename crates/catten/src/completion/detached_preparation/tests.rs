//! Private original accounts and real prepared timer nodes; rejected borrowed
//! publication must retain its fields beyond both registry and heap guards.

use core::alloc::AllocError;

use super::*;
use crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR;

const CLIENT: usize = 0x5e35;

fn unlocked_preparation() {
    open_address_space_with_cq(CLIENT, 4, 4).unwrap();
    let records = record_admission(CLIENT).unwrap();
    for mode in 0..3 {
        let prepared = Preparing::timer_with(
            Target::capture(CLIENT, DEFAULT_CQ).unwrap(),
            u64::MAX,
            1,
            |event| {
                {
                    let _registry = COMPLETIONS.write();
                }
                if mode == 0 {
                    Err(())
                } else {
                    crate::timers::PreparedEvent::new(event)
                }
            },
            |observer| {
                {
                    let registry = COMPLETIONS.write();
                    assert!(registry[&CLIENT].detached.is_empty());
                }
                if mode == 1 {
                    Err(AllocError)
                } else {
                    Arc::try_new(observer)
                }
            },
        );
        if mode < 2 {
            assert!(matches!(prepared, Err(SubmitError::WouldBlock)));
        } else {
            let prepared = prepared.unwrap();
            assert_eq!(records.used(), 1);
            assert_eq!(timer_events_used(CLIENT), 1);
            assert!(COMPLETIONS.read()[&CLIENT].detached.is_empty());
            assert!(prepared.record.is_some() && prepared.event.is_some());
            drop(prepared);
        }
        assert_eq!(records.used(), 0);
        assert_eq!(timer_events_used(CLIENT), 0);
        assert_eq!(cq_pending(CLIENT, DEFAULT_CQ), 0);
    }
    // The actual public adapter uses the same unlocked observer constructor.
    let operation =
        submit_detached_timer_with_observer(CLIENT, DEFAULT_CQ, u64::MAX, 2, |observer| {
            {
                let registry = COMPLETIONS.write();
                assert!(registry[&CLIENT].detached.is_empty());
            }
            Arc::try_new(observer)
        })
        .unwrap();
    assert_eq!(cancel_detached(CLIENT, operation), Ok(CancelState::CancelRequested));
    assert_eq!(records.used(), 0);
    assert_eq!(timer_events_used(CLIENT), 0);
    let ring = unsafe { cq_ring_of(CLIENT, DEFAULT_CQ).unwrap() };
    // Fixture is the only consumer of this private heap ring.
    let entry = unsafe { (&mut *ring).read().unwrap() };
    assert_eq!(entry.operation, operation);
    assert_eq!(entry.cookie, 2);
    assert_eq!(cq::fields_to_op_result(entry.status, entry.result), OpResult::Cancelled);
    let operation = submit_detached(CLIENT, DEFAULT_CQ, OpCode::Nop, 3).unwrap();
    complete_detached(CLIENT, operation, OpResult::Ok(3)).unwrap();
    assert_eq!(unsafe { (&mut *ring).read().unwrap() }.cookie, 3);
    assert_eq!(records.used(), 0);
    close_address_space(CLIENT);
}

fn prepared_admission() {
    open_address_space_with_cq(CLIENT, 2, 4).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let timer =
        Preparing::timer(Target::capture(CLIENT, DEFAULT_CQ).unwrap(), u64::MAX, 4, Arc::try_new)
            .unwrap();
    let plain = Preparing::plain(Target::capture(CLIENT, DEFAULT_CQ).unwrap(), 5).unwrap();
    assert_eq!(records.used(), 2);
    assert_eq!(timer_events_used(CLIENT), 1);
    assert_eq!(submit_detached_timer(CLIENT, DEFAULT_CQ, 0, 6), Err(SubmitError::WouldBlock));
    assert!(COMPLETIONS.read()[&CLIENT].detached.is_empty());
    assert_eq!(timer_events_used(CLIENT), 1);
    drop(plain);
    assert_eq!(records.used(), 1);
    drop(timer);
    assert_eq!(records.used(), 0);
    assert_eq!(timer_events_used(CLIENT), 0);
    close_address_space(CLIENT);
}

fn rejected_publication() {
    for mode in 0..4 {
        let root = if mode == 2 {
            Some(crate::service::loader::create_user_address_space_handle())
        } else {
            None
        };
        let asid = root.map_or(CLIENT, |root| root.id());
        open_address_space_with_cq(asid, 4, 4).unwrap();
        let records = record_admission(asid).unwrap();
        let timers = COMPLETIONS.read()[&asid].timer_budget.clone();
        let mut timer =
            Preparing::timer(Target::capture(asid, DEFAULT_CQ).unwrap(), u64::MAX, 7, Arc::try_new)
                .unwrap();
        let mut plain = Preparing::plain(Target::capture(asid, DEFAULT_CQ).unwrap(), 8).unwrap();
        match mode {
            0 => open_address_space_with_cq(asid, 4, 4).unwrap(),
            1 => open_cq(asid, DEFAULT_CQ, 4).unwrap(),
            2 => crate::memory::budget::retire(root.unwrap()),
            _ => {
                let queue = COMPLETIONS.write().get_mut(&asid).unwrap().cqs.remove(&DEFAULT_CQ);
                drop(queue);
            }
        }
        let successor_records = record_admission(asid).unwrap();
        let successor_timers = COMPLETIONS.read()[&asid].timer_budget.clone();
        let before = (successor_records.used(), successor_timers.used());
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            for preparing in [&mut timer, &mut plain] {
                let error = preparing.publish().unwrap_err();
                assert_eq!(
                    error,
                    if mode == 0 || mode == 2 {
                        SubmitError::UnknownAddressSpace
                    } else {
                        SubmitError::NoCompletionQueue
                    }
                );
                assert!(preparing.record.is_some());
            }
            assert!(timer.event.is_some());
            assert_eq!(records.used(), 2);
            assert_eq!(timers.used(), 1);
            assert_eq!((successor_records.used(), successor_timers.used()), before);
            assert!(COMPLETIONS.read()[&asid].detached.is_empty());
        }
        drop((timer, plain));
        assert_eq!(records.used(), 0);
        assert_eq!(timers.used(), 0);
        if let Some(root) = root {
            crate::memory::close_user_address_space_handle(root).unwrap();
        } else {
            close_address_space(asid);
        }
    }
}

fn close_before_enqueue() {
    open_address_space_with_cq(CLIENT, 4, 4).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let timers = COMPLETIONS.read()[&CLIENT].timer_budget.clone();
    let mut preparing =
        Preparing::timer(Target::capture(CLIENT, DEFAULT_CQ).unwrap(), 0, 9, Arc::try_new).unwrap();
    let operation = preparing.publish().unwrap();
    assert!(preparing.record.is_none());
    assert_eq!(records.used(), 1);
    assert_eq!(timers.used(), 1);
    close_address_space(CLIENT);
    assert_eq!(records.used(), 0);
    assert_eq!(timers.used(), 1, "unpublished prepared node still owns its original event charge");
    preparing.enqueue();
    assert_eq!(timers.used(), 0);
    assert_eq!(
        complete_detached(CLIENT, operation, OpResult::Ok(0)),
        Err(CapError::UnknownAddressSpace)
    );
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    unlocked_preparation();
    prepared_admission();
    rejected_publication();
    close_before_enqueue();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[detached preparation] unlocked node/observer allocation and rejection, bounded prepared \
         charges, heap-held stale/replaced/missing/retired publication, public recovery and \
         close-before-enqueue retention passed"
    );
}
