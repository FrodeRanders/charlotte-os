//! Serialized early admission controls, before record/authority publication.

use super::*;
use crate::{
    cpu::{
        isa::lp::ops::get_int_state,
        multiprocessor::interrupt_tracking::LocalInterruptMask,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

const CLIENT: usize = 0x5e3b;

fn context() {
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(COMPLETIONS.try_write().is_some());
}

fn ordinary_rejection() {
    for mode in 0..4 {
        open_address_space_with_cq(CLIENT, 4, 8).unwrap();
        let records = record_admission(CLIENT).unwrap();
        let watches = watch_admission(CLIENT).unwrap();
        let authority = crate::capability::node_admission_used();
        let mut record_pressure = Vec::new();
        let mut watch_pressure = Vec::new();
        if mode == 1 {
            while let Ok(charge) = budget::reserve(&records, false) {
                record_pressure.push(charge);
            }
        }
        if mode == 2 {
            while let Ok(charge) = watch_budget::reserve(&watches, false) {
                watch_pressure.push(charge);
            }
        }
        let before = (records.used(), watches.used());
        let ids = operation_ids::OperationIds::new(
            if mode == 0 {
                operation_ids::LAST_OPERATION_ID
            } else {
                0
            },
        );
        let result = Preparing::new_with(
            Target::capture(CLIENT).unwrap(),
            Some(alloc::vec![0x51; 16]),
            true,
            &ids,
            |preparing| {
                assert_eq!(mode, 3, "rejected admission reached record factory");
                let _heap = PRIMARY_ALLOCATOR.lock();
                let _registry = COMPLETIONS.write();
                assert_eq!(preparing.0.buffer.as_deref(), Some(&[0x51; 16][..]));
                assert!(preparing.0.record_charge.is_some() && preparing.0.watch.is_some());
                assert_eq!((records.used(), watches.used()), (1, 1));
                Err(SubmitError::WouldBlock)
            },
            |_, _| panic!("early rejection reached authority allocation"),
            || panic!("early rejection reached metadata allocation"),
        );
        assert!(matches!(result, Err(SubmitError::WouldBlock)));
        assert_eq!((records.used(), watches.used()), before);
        assert_eq!(crate::capability::node_admission_used(), authority);
        assert_eq!(COMPLETIONS.read()[&CLIENT].live, 0);
        assert_eq!(cq_pending(CLIENT, DEFAULT_CQ), 0);
        // Identity rejection occurs first; every later rejection consumes its
        // admitted operation exactly once without resetting the private counter.
        assert_eq!(
            ids.allocate(),
            if mode == 0 {
                None
            } else {
                Some(2)
            }
        );
        drop((record_pressure, watch_pressure));
        assert_eq!((records.used(), watches.used()), (0, 0));
        close_address_space(CLIENT);
        crate::capability::close_address_space(CLIENT);
    }
}

fn retained_watch_rejection() {
    open_address_space_with_cq(CLIENT, 1, 4).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let watches = watch_admission(CLIENT).unwrap();
    let pressure = watch_budget::reserve(&watches, false).unwrap();
    let ids = operation_ids::OperationIds::new(0);
    let mut preparing =
        Preparing::capture(Target::capture(CLIENT).unwrap(), Some(alloc::vec![0x52; 16]));
    let pointer = preparing.0.buffer.as_ref().unwrap().as_ptr();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _registry = COMPLETIONS.write();
        assert_eq!(preparing.admit(true, &ids), Err(SubmitError::WouldBlock));
        assert!(preparing.0.record_charge.is_some() && preparing.0.watch.is_none());
        assert_eq!(preparing.0.buffer.as_ref().unwrap().as_ptr(), pointer);
        assert_eq!(preparing.0.buffer.as_deref(), Some(&[0x52; 16][..]));
        assert_eq!((records.used(), watches.used()), (1, 1));
    }
    let mut checks = 0;
    preparing.finish_with(|| {
        context();
        assert_eq!((records.used(), watches.used()), (1, 1));
        checks += 1;
    });
    assert_eq!(checks, 2);
    assert_eq!((records.used(), watches.used()), (0, 1));
    drop(pressure);
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
}

fn borrowed_success() {
    open_address_space_with_cq(CLIENT, 4, 8).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let watches = watch_admission(CLIENT).unwrap();
    let ids = operation_ids::OperationIds::new(operation_ids::LAST_OPERATION_ID - 64);
    for _ in 0..64 {
        let buffer = alloc::vec![0x53; 16];
        let pointer = buffer.as_ptr();
        let mut preparing = Preparing::new_with(
            Target::capture(CLIENT).unwrap(),
            Some(buffer),
            true,
            &ids,
            |preparing| {
                context();
                assert!(preparing.0.buffer.is_some() && preparing.0.record_charge.is_some());
                assert!(preparing.0.watch.is_some());
                assert_eq!((records.used(), watches.used()), (1, 1));
                let operation = preparing.0.operation.unwrap();
                preparing.record()?;
                assert!(preparing.0.buffer.is_none() && preparing.0.record_charge.is_none());
                let completion = preparing.0.completion.as_ref().unwrap();
                assert_eq!(completion.operation_id(), operation);
                assert_eq!(completion.inner.lock().buffer.as_ref().unwrap().as_ptr(), pointer);
                Ok(())
            },
            |asid, root| PreparedReservation::try_new(asid, ObjectKind::Completion, root),
            PreparedEntry::try_new,
        )
        .unwrap();
        let operation = preparing.0.operation.unwrap();
        let cap = preparing.publish().unwrap();
        preparing.finish();
        assert_eq!((records.used(), watches.used()), (1, 0));
        assert_eq!(operation_id(CLIENT, cap).unwrap(), operation);
        complete(CLIENT, cap, OpResult::Ok(16)).unwrap();
        let done = poll(CLIENT, cap).unwrap().unwrap();
        assert_eq!(done.buffer.as_ref().unwrap().as_ptr(), pointer);
        assert_eq!(done.buffer.as_deref(), Some(&[0x53; 16][..]));
        close(CLIENT, cap).unwrap();
        assert_eq!((records.used(), watches.used()), (0, 0));
    }
    assert!(ids.allocate().is_none());
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
}

#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    open_address_space_with_cq(CLIENT, 4, 8).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let watches = watch_admission(CLIENT).unwrap();
    let authority = crate::capability::node_admission_used();
    let ids = operation_ids::OperationIds::new(0);
    for phase in 0..3 {
        let mut preparing =
            Preparing::capture(Target::capture(CLIENT).unwrap(), Some(alloc::vec![0x54; 16]));
        if phase >= 1 {
            preparing.admit(phase == 2, &ids).unwrap();
        }
        let mask = LocalInterruptMask::new();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _registry = COMPLETIONS.write();
            drop(preparing);
            assert_eq!((records.used(), watches.used()), (phase, usize::from(phase == 2)));
        }
        drop(mask);
    }
    assert_eq!(crate::capability::node_admission_used(), authority);
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    assert_eq!((records.used(), watches.used()), (2, 1));
    open_address_space_with_cq(CLIENT, 4, 8).unwrap();
    assert_eq!(record_admission(CLIENT).unwrap().used(), 0);
    assert_eq!(watch_admission(CLIENT).unwrap().used(), 0);
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    assert_eq!((records.used(), watches.used()), (2, 1));
    crate::logln!(
        "[completion initial admission retention] guarded captured-buffer, record and watch \
         phases retained three 16-byte buffers, two original record charges, one watch charge and \
         captured accounts across namespace replacement; no authority or user root"
    );
}

pub(super) fn run() {
    let irq = get_int_state();
    let records = budget::node_used();
    let watches = watch_budget::node_used();
    ordinary_rejection();
    retained_watch_rejection();
    borrowed_success();
    assert_eq!(budget::node_used(), records);
    assert_eq!(watch_budget::node_used(), watches);
    guarded_abandonment();
    assert_eq!(budget::node_used(), (records.0 + 2, records.1 + 2));
    assert_eq!(watch_budget::node_used(), (watches.0 + 1, watches.1 + 1));
    assert_eq!(get_int_state(), irq);
    crate::logln!(
        "[completion initial admission] identity/record/watch/factory rejection, retained \
         pre-record buffer and original charge, 64 borrowed factory publication/result transfers \
         and explicit payload-first rollback passed"
    );
}
