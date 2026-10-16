//! Serialized original-account controls. Heap-held rejected publication must
//! retain payload, hidden reservation and every unused preparation field.

use super::*;
use crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR;

const CLIENT: usize = 0x5e37;

fn unlocked() {
    open_address_space_with_cq(CLIENT, 4, 8).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let watches = watch_admission(CLIENT).unwrap();
    let caps = crate::capability::node_admission_used();
    for mode in 0..3 {
        let prepared = Preparing::new_with(
            Target::capture(CLIENT).unwrap(),
            Some(alloc::vec![1, 2, 3]),
            true,
            &OPERATION_IDS,
            |operation, buffer, charge| {
                assert!(COMPLETIONS.write()[&CLIENT].table.is_empty());
                if mode == 0 {
                    Err(SubmitError::WouldBlock)
                } else {
                    Completion::new(operation, buffer, charge)
                }
            },
            |asid, root| {
                assert!(COMPLETIONS.write()[&CLIENT].table.is_empty());
                if mode == 1 {
                    // Boot fixture is serialized; no live sibling can consume
                    // this existing one-shot authority-node allocation hook.
                    crate::capability::record_tests::reject_next();
                }
                PreparedReservation::try_new(asid, ObjectKind::Completion, root)
            },
        );
        if mode < 2 {
            assert!(matches!(prepared, Err(SubmitError::WouldBlock)));
        } else {
            let prepared = prepared.unwrap();
            let cap = prepared.0.reservation.as_ref().unwrap().identity();
            assert!(!crate::capability::contains(CLIENT, cap, ObjectKind::Completion));
            assert_eq!((records.used(), watches.used()), (1, 1));
            assert_eq!(COMPLETIONS.read()[&CLIENT].live, 0);
            prepared.finish();
        }
        assert_eq!((records.used(), watches.used()), (0, 0));
        assert_eq!(crate::capability::node_admission_used(), caps);
    }

    for mode in 0..3 {
        let mut preparing =
            Preparing::new(Target::capture(CLIENT).unwrap(), None, false, &OPERATION_IDS).unwrap();
        let result = preparing.timer(
            0,
            |event| {
                assert!(COMPLETIONS.write()[&CLIENT].table.is_empty());
                assert_eq!(event.get_deadline(), 0, "expired deadline was restarted");
                if mode == 0 {
                    Err(())
                } else {
                    crate::timers::PreparedEvent::new(event)
                }
            },
            |observer| {
                assert!(COMPLETIONS.write()[&CLIENT].table.is_empty());
                assert!(!crate::capability::contains(CLIENT, observer.cap, ObjectKind::Completion));
                if mode == 1 {
                    Err(core::alloc::AllocError)
                } else {
                    Arc::try_new(observer)
                }
            },
        );
        if mode < 2 {
            assert_eq!(result, Err(SubmitError::WouldBlock));
        } else {
            result.unwrap();
            assert_eq!(preparing.0.event.as_ref().unwrap().event().get_deadline(), 0);
        }
        preparing.finish();
        assert_eq!(records.used(), 0);
        assert_eq!(timer_events_used(CLIENT), 0);
        assert_eq!(crate::capability::node_admission_used(), caps);
    }
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
}

fn reject_before_reservation() {
    for mode in 0..3 {
        let root = if mode == 2 {
            Some(crate::service::loader::create_user_address_space_handle())
        } else {
            None
        };
        let asid = root.map_or(CLIENT, |root| root.id());
        open_address_space_with_cq(asid, 4, 8).unwrap();
        let records = record_admission(asid).unwrap();
        let watches = watch_admission(asid).unwrap();
        let caps = crate::capability::node_admission_used();
        let result = Preparing::new_with(
            Target::capture(asid).unwrap(),
            None,
            true,
            &OPERATION_IDS,
            Completion::new,
            |asid, captured| {
                let authority =
                    PreparedReservation::try_new(asid, ObjectKind::Completion, captured)?;
                match mode {
                    0 => open_address_space_with_cq(asid, 4, 8).unwrap(),
                    1 => close_address_space(asid),
                    _ => crate::memory::budget::retire(root.unwrap()),
                }
                Ok(authority)
            },
        );
        assert!(matches!(result, Err(SubmitError::UnknownAddressSpace)));
        assert_eq!((records.used(), watches.used()), (0, 0));
        assert_eq!(crate::capability::node_admission_used(), caps);
        if let Some(root) = root {
            crate::memory::close_user_address_space_handle(root).unwrap();
        } else {
            close_address_space(asid);
            crate::capability::close_address_space(asid);
        }
    }
}

fn reject_after_reservation() {
    for mode in 0..4 {
        let root = if mode == 2 {
            Some(crate::service::loader::create_user_address_space_handle())
        } else {
            None
        };
        let asid = root.map_or(CLIENT, |root| root.id());
        open_address_space_with_cq(asid, 4, 8).unwrap();
        let records = record_admission(asid).unwrap();
        let watches = watch_admission(asid).unwrap();
        let timers = COMPLETIONS.read()[&asid].timer_budget.clone();
        let mut plain = Preparing::new(
            Target::capture(asid).unwrap(),
            Some(alloc::vec![4, 5]),
            true,
            &OPERATION_IDS,
        )
        .unwrap();
        let mut timer =
            Preparing::new(Target::capture(asid).unwrap(), None, false, &OPERATION_IDS).unwrap();
        timer.timer(0, crate::timers::PreparedEvent::new, Arc::try_new).unwrap();
        let mut successor = None;
        match mode {
            0 => open_address_space_with_cq(asid, 4, 8).unwrap(),
            1 => close_address_space(asid),
            2 => crate::memory::budget::retire(root.unwrap()),
            _ => {
                crate::capability::close_address_space(asid);
                successor = Some(crate::capability::reserve(asid, ObjectKind::Completion).unwrap());
                assert_eq!(
                    successor.as_ref().unwrap().identity(),
                    plain.0.reservation.as_ref().unwrap().identity()
                );
            }
        }
        let caps = crate::capability::node_admission_used();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            for preparing in [&mut plain, &mut timer] {
                assert_eq!(
                    preparing.publish(),
                    Err(if mode == 3 {
                        SubmitError::WouldBlock
                    } else {
                        SubmitError::UnknownAddressSpace
                    })
                );
                assert!(!preparing.0.published);
                assert!(preparing.0.completion.is_some() && preparing.0.reservation.is_some());
                assert!(!crate::capability::contains(
                    asid,
                    preparing.0.reservation.as_ref().unwrap().identity(),
                    ObjectKind::Completion
                ));
            }
            assert!(timer.0.event.is_some());
            assert_eq!((records.used(), watches.used(), timers.used()), (2, 1, 1));
            assert_eq!(crate::capability::node_admission_used(), caps);
        }
        plain.finish();
        timer.finish();
        assert_eq!((records.used(), watches.used(), timers.used()), (0, 0, 0));
        if let Some(successor) = successor {
            let cap = successor.publish().unwrap();
            assert!(crate::capability::contains(asid, cap, ObjectKind::Completion));
            assert!(crate::capability::remove(asid, cap, ObjectKind::Completion));
        }
        if let Some(root) = root {
            crate::memory::close_user_address_space_handle(root).unwrap();
        } else {
            close_address_space(asid);
            crate::capability::close_address_space(asid);
        }
    }
}

fn accepted_timer_close_and_deadline() {
    open_address_space_with_cq(CLIENT, 4, 8).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let timers = COMPLETIONS.read()[&CLIENT].timer_budget.clone();
    let mut preparing =
        Preparing::new(Target::capture(CLIENT).unwrap(), None, false, &OPERATION_IDS).unwrap();
    preparing.timer(0, crate::timers::PreparedEvent::new, Arc::try_new).unwrap();
    let cap = preparing.publish().unwrap();
    assert!(crate::capability::contains(CLIENT, cap, ObjectKind::Completion));
    close_address_space(CLIENT);
    assert_eq!((records.used(), timers.used()), (1, 1));
    open_address_space_with_cq(CLIENT, 4, 8).unwrap();
    preparing.enqueue();
    preparing.finish();
    crate::timers::process_local_events();
    assert_eq!((records.used(), timers.used()), (0, 0));
    assert_eq!(cq_pending(CLIENT, DEFAULT_CQ), 0, "old timer reached replacement CQ");

    let mut preparing =
        Preparing::new(Target::capture(CLIENT).unwrap(), None, false, &OPERATION_IDS).unwrap();
    preparing.timer(0, crate::timers::PreparedEvent::new, Arc::try_new).unwrap();
    let cap = preparing.publish().unwrap();
    assert_eq!(preparing.0.event.as_ref().unwrap().event().get_deadline(), 0);
    preparing.enqueue();
    preparing.finish();
    crate::timers::process_local_events();
    assert_eq!(poll(CLIENT, cap).unwrap().unwrap().result, OpResult::Ok(0));
    close(CLIENT, cap).unwrap();
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    unlocked();
    reject_before_reservation();
    reject_after_reservation();
    accepted_timer_close_and_deadline();
    backlog_refund_on_publication_rejection();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[capability completion preparation] unlocked construction/rejection, hidden authority, \
         heap-held stale publication, original charges, absolute deadline and \
         close-before-enqueue passed"
    );
}

fn backlog_refund_on_publication_rejection() {
    for timer in [false, true] {
        open_address_space_with_cq(CLIENT, 4, 2).unwrap();
        let records = record_admission(CLIENT).unwrap();
        let mut preparing =
            Preparing::new(Target::capture(CLIENT).unwrap(), None, false, &OPERATION_IDS).unwrap();
        if timer {
            preparing.timer(0, crate::timers::PreparedEvent::new, Arc::try_new).unwrap();
        }
        for cookie in 1..=2 {
            let op = submit_detached(CLIENT, DEFAULT_CQ, OpCode::Nop, cookie).unwrap();
            complete_detached(CLIENT, op, OpResult::Ok(cookie as i64)).unwrap();
        }
        assert_eq!(records.used(), 2);
        let ring = unsafe { cq_ring_of(CLIENT, DEFAULT_CQ).unwrap() };
        assert_eq!(unsafe { (&mut *ring).read().unwrap() }.cookie, 1);
        // The captured authority now rejects after a real backlog flush. The
        // flush's charges must return after unlock even on this error path;
        // the rejected operation itself remains completely owned.
        crate::capability::close_address_space(CLIENT);
        assert_eq!(preparing.publish(), Err(SubmitError::WouldBlock));
        assert_eq!(records.used(), 1);
        assert_eq!(timer_events_used(CLIENT), usize::from(timer));
        assert!(preparing.0.completion.is_some() && preparing.0.reservation.is_some());
        assert_eq!(unsafe { (&mut *ring).read().unwrap() }.cookie, 2);
        preparing.finish();
        assert_eq!(records.used(), 0);
        assert_eq!(timer_events_used(CLIENT), 0);
        close_address_space(CLIENT);
    }
}
