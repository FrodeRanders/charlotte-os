//! Timer producer-observer allocation rejection before publication.

use core::alloc::AllocError;

use super::{
    CancelState,
    OpCode,
    OpResult,
    OpStateKind,
    SubmitError,
};
use crate::{
    klib::observer::list_budget,
    timers,
};

const CLIENT: usize = 0xc0ae_b006;

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    capabilities: (usize, usize),
    records: (usize, usize),
    events: (usize, usize),
    lists: (usize, usize),
    live: usize,
    cap_records: usize,
    detached_records: usize,
    backlog: usize,
}

fn snapshot() -> Snapshot {
    let (live, cap_records, detached_records, backlog) = {
        let registry = super::COMPLETIONS.read();
        let entries = registry.get(&CLIENT).unwrap();
        (
            entries.live,
            entries.table.len(),
            entries.detached.len(),
            entries.cqs.get(&super::DEFAULT_CQ).unwrap().backlog.len(),
        )
    };
    Snapshot {
        capabilities: crate::capability::node_admission_used(),
        records: super::budget::node_used(),
        events: timers::budget::node_used(),
        lists: list_budget::node_used(),
        live,
        cap_records,
        detached_records,
        backlog,
    }
}

/// Uses the production preparation/publication paths with only the final
/// observer allocator substituted. This is not whole-heap exhaustion.
pub(crate) fn test_observer_allocation_rollback() {
    super::open_address_space_with_cq(CLIENT, 2, 4).unwrap();
    let account = super::record_admission(CLIENT).unwrap();
    let pending = super::submit(CLIENT, OpCode::Nop, None).unwrap();
    let baseline = snapshot();
    let ring = unsafe { super::cq_ring_of(CLIENT, super::DEFAULT_CQ) }.unwrap();
    for _ in 0..64 {
        assert_eq!(
            super::submit_timer_with_observer(CLIENT, u64::MAX, |_| Err(AllocError)),
            Err(SubmitError::WouldBlock)
        );
        assert_eq!(snapshot(), baseline);
        assert_eq!(
            super::submit_detached_timer_with_observer(
                CLIENT,
                super::DEFAULT_CQ,
                u64::MAX,
                0xbad,
                |_| Err(AllocError),
            ),
            Err(SubmitError::WouldBlock)
        );
        assert_eq!(snapshot(), baseline);
        assert_eq!(account.used(), 1);
        assert_eq!(super::timer_events_used(CLIENT), 0);
        assert_eq!(super::state_of(CLIENT, pending), Ok(OpStateKind::InFlight));
        // This fixture owns the pseudo namespace's only CQ consumer.
        assert!(unsafe { &mut *ring }.read().is_none());
    }

    // A deliberately retained weak reference outlives rejected preparation.
    // The dead record must stay charged until that reference is released.
    let mut retained = None;
    assert_eq!(
        super::submit_timer_with_observer(CLIENT, u64::MAX, |observer| {
            retained = Some(observer.completion.clone());
            Err(AllocError)
        }),
        Err(SubmitError::WouldBlock)
    );
    let retained = retained.unwrap();
    assert!(retained.upgrade().is_none());
    assert_eq!(account.used(), 2);
    assert_eq!(super::timer_events_used(CLIENT), 0);
    assert_eq!(super::submit_timer(CLIENT, 0), Err(SubmitError::WouldBlock));
    drop(retained);
    assert_eq!(snapshot(), baseline);

    // Real immediate producers still publish and notify after failure, using
    // the same remaining slot while unrelated pending work stays live.
    let cap = super::submit_timer(CLIENT, 0).unwrap();
    let operation = super::operation_id(CLIENT, cap).unwrap();
    timers::process_local_events();
    assert_eq!(super::poll(CLIENT, cap).unwrap().unwrap().result, OpResult::Ok(0));
    super::close(CLIENT, cap).unwrap();
    let delivered = unsafe { &mut *ring }.read().unwrap();
    assert_eq!(delivered.operation, operation);
    assert_eq!(delivered.cookie, cap);
    assert_eq!(super::cq::fields_to_op_result(delivered.status, delivered.result), OpResult::Ok(0));

    let operation = super::submit_detached_timer(CLIENT, super::DEFAULT_CQ, 0, 0x77).unwrap();
    timers::process_local_events();
    let delivered = unsafe { &mut *ring }.read().unwrap();
    assert_eq!(delivered.operation, operation);
    assert_eq!(delivered.cookie, 0x77);
    assert_eq!(super::cq::fields_to_op_result(delivered.status, delivered.result), OpResult::Ok(0));
    assert!(unsafe { &mut *ring }.read().is_none());
    assert_eq!(snapshot(), baseline);
    assert_eq!(super::state_of(CLIENT, pending), Ok(OpStateKind::InFlight));

    let cap = super::submit_timer(CLIENT, u64::MAX).unwrap();
    assert_eq!(super::cancel(CLIENT, cap), Ok(CancelState::CancelRequested));
    super::close(CLIENT, cap).unwrap();
    super::abort_submission(CLIENT, pending).unwrap();
    assert_eq!(account.used(), 0);
    assert_eq!(super::timer_events_used(CLIENT), 0);
    super::close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    crate::logln!(
        "[completion timer observers] allocation rejection, rollback and recovery passed"
    );
}
