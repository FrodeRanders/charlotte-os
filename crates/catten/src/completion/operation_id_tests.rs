//! Exhaust real submission paths using private counters, never the live
//! boot-lifetime allocator. Rejection must precede charge/authority/timer work.

use operation_ids::{
    LAST_OPERATION_ID,
    OperationIds,
};

use super::*;

const CLIENT: usize = 0x5e36;

fn reject_submissions(ids: &OperationIds) {
    for watch in [false, true] {
        assert!(matches!(
            submit_captured(CLIENT, Some(alloc::vec![1, 2, 3]), watch, ids),
            Err(SubmitError::WouldBlock)
        ));
    }
    assert_eq!(
        submit_timer_with_observer(CLIENT, 0, ids, |_| {
            panic!("exhausted operation IDs reached timer observer allocation")
        }),
        Err(SubmitError::WouldBlock)
    );
    assert_eq!(
        detached_preparation::submit(CLIENT, DEFAULT_CQ, 9, ids),
        Err(SubmitError::WouldBlock)
    );
    assert_eq!(
        detached_preparation::submit_timer(CLIENT, DEFAULT_CQ, 0, 10, ids, |_| {
            panic!("exhausted operation IDs reached detached observer allocation")
        }),
        Err(SubmitError::WouldBlock)
    );
}

fn boundary_and_replacement() {
    open_address_space_with_cq(CLIENT, 4, 8).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let watches = watch_admission(CLIENT).unwrap();
    let ids = OperationIds::new(LAST_OPERATION_ID - 2);
    let (cap, completion, watch) =
        submit_captured(CLIENT, Some(alloc::vec![4, 5, 6]), false, &ids).unwrap();
    assert!(watch.is_none());
    assert_eq!(completion.operation_id(), LAST_OPERATION_ID - 1);
    let detached = detached_preparation::submit(CLIENT, DEFAULT_CQ, 7, &ids).unwrap();
    assert_eq!(detached, LAST_OPERATION_ID);

    for _ in 0..16 {
        reject_submissions(&ids);
        assert_eq!(records.used(), 2);
        assert_eq!(watches.used(), 0);
        assert_eq!(timer_events_used(CLIENT), 0);
        assert_eq!(cq_pending(CLIENT, DEFAULT_CQ), 0);
        assert_eq!(state_of(CLIENT, cap), Ok(OpStateKind::InFlight));
        let registry = COMPLETIONS.read();
        let entries = &registry[&CLIENT];
        assert_eq!((entries.live, entries.table.len(), entries.detached.len()), (2, 1, 1));
        assert!(Arc::ptr_eq(&entries.table[&cap], &completion));
        assert!(entries.detached.contains_key(&detached));
    }

    // Existing authority and producer results remain usable after exhaustion.
    complete(CLIENT, cap, OpResult::Ok(3)).unwrap();
    let done = poll(CLIENT, cap).unwrap().unwrap();
    assert_eq!(done.buffer.as_deref(), Some(&[4, 5, 6][..]));
    close(CLIENT, cap).unwrap();
    drop(completion);
    complete_detached(CLIENT, detached, OpResult::Ok(7)).unwrap();
    assert_eq!(records.used(), 0);
    close_address_space(CLIENT);

    open_address_space_with_cq(CLIENT, 4, 8).unwrap();
    let successor = record_admission(CLIENT).unwrap();
    assert!(!Arc::ptr_eq(&records, &successor));
    reject_submissions(&ids);
    assert_eq!(successor.used(), 0);
    assert_eq!(timer_events_used(CLIENT), 0);
    assert_eq!(watches.used(), 0);
    {
        let registry = COMPLETIONS.read();
        let entries = &registry[&CLIENT];
        assert_eq!((entries.live, entries.table.len(), entries.detached.len()), (0, 0, 0));
    }
    // The actual global counter was never changed by this boundary fixture.
    let cap = submit(CLIENT, OpCode::Nop, None).unwrap();
    assert_ne!(operation_id(CLIENT, cap).unwrap(), 0);
    complete(CLIENT, cap, OpResult::Ok(0)).unwrap();
    close(CLIENT, cap).unwrap();
    close_address_space(CLIENT);
}

fn rejected_preparation_consumes_identity() {
    open_address_space_with_cq(CLIENT, 4, 8).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let ids = OperationIds::new(LAST_OPERATION_ID - 2);
    assert_eq!(
        submit_timer_with_observer(CLIENT, u64::MAX, &ids, |_| Err(core::alloc::AllocError)),
        Err(SubmitError::WouldBlock)
    );
    assert_eq!(records.used(), 0);
    assert_eq!(timer_events_used(CLIENT), 0);
    let operation = detached_preparation::submit(CLIENT, DEFAULT_CQ, 11, &ids).unwrap();
    assert_eq!(operation, LAST_OPERATION_ID);
    reject_submissions(&ids);
    complete_detached(CLIENT, operation, OpResult::Ok(11)).unwrap();
    assert_eq!(records.used(), 0);
    close_address_space(CLIENT);
}

pub(super) fn run() {
    assert_eq!(LAST_OPERATION_ID + 1, catten_syscall::COMPLETION_SUBMIT_FAILED);
    boundary_and_replacement();
    rejected_preparation_consumes_identity();
    crate::logln!("Operation-ID exhaustion controls passed (isolated counters).");
}
