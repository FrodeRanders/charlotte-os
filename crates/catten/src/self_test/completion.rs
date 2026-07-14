//! Self-tests for the completion-capability subsystem (Option C prototype).
//!
//! These are whitebox integration tests of the kernel side of the async syscall
//! ABI (`docs/async-syscall-abi.md`). They validate the submission-side
//! semantics that exist today — the capability table, the buffer-ownership /
//! deferred-reclaim contract, the observer-signal path that [`wait`] relies on,
//! and submission backpressure — without needing the (not-yet-existing) EL0
//! syscall entry or a running scheduler.
//!
//! [`wait`](crate::completion::wait) itself is not exercised here because it
//! blocks the calling thread, which requires the scheduler to be yielding;
//! self-tests run before the BSP yields. The signal path `wait` depends on is
//! validated directly via [`observe`](crate::completion::observe).

use crate::{
    completion::{
        self,
        CancelState,
        OpCode,
        OpResult,
        OpStateKind,
        SubmitError,
    },
    logln,
};

pub fn test_completion_caps() {
    logln!("Testing completion-capability subsystem...");

    let asid = 0xc0ffee;
    completion::open_address_space(asid, 2);

    // --- submit transfers buffer ownership to the kernel ---------------------
    let cap = completion::submit(asid, OpCode::Read, Some(alloc::vec![0u8; 4])).unwrap();
    assert!(completion::holds_buffer(asid, cap).unwrap());
    assert_eq!(completion::state_of(asid, cap).unwrap(), OpStateKind::InFlight);

    // --- complete posts the result, hands the buffer back on poll -----------
    completion::complete(asid, cap, OpResult::Ok(4)).unwrap();
    assert_eq!(completion::state_of(asid, cap).unwrap(), OpStateKind::Completed);
    // Buffer stays with the kernel until poll() drains it.
    assert!(completion::holds_buffer(asid, cap).unwrap());

    // A second completion is an idempotent no-op and must not change the result.
    completion::complete(asid, cap, OpResult::Err(9)).unwrap();

    let done = completion::poll(asid, cap).unwrap().expect("must be complete");
    assert!(!completion::holds_buffer(asid, cap).unwrap());
    assert!(matches!(done.result, OpResult::Ok(4)));
    assert_eq!(done.buffer.as_deref(), Some(&[0u8; 4][..]));
    assert_eq!(completion::state_of(asid, cap).unwrap(), OpStateKind::Observed);

    // Draining twice yields nothing; the observed state is stable.
    assert!(completion::poll(asid, cap).unwrap().is_none());
    assert_eq!(completion::state_of(asid, cap).unwrap(), OpStateKind::Observed);
    // Completing an observed operation is rejected as a no-op.
    completion::complete(asid, cap, OpResult::Ok(1)).unwrap();
    assert_eq!(completion::state_of(asid, cap).unwrap(), OpStateKind::Observed);
    // Cancelling an observed operation reports AlreadyComplete.
    assert_eq!(completion::cancel(asid, cap).unwrap(), CancelState::AlreadyComplete);

    let first_operation = completion::operation_id(asid, cap).unwrap();

    // --- close frees the slot ------------------------------------------------
    completion::close(asid, cap).unwrap();
    assert!(completion::poll(asid, cap).is_err());

    // --- operation ids are stable identity across capability-slot reuse ------
    let cap_reused = completion::submit(asid, OpCode::Nop, None).unwrap();
    assert_eq!(cap_reused, cap, "IdTable should reuse the freed capability slot");
    assert_ne!(
        completion::operation_id(asid, cap_reused).unwrap(),
        first_operation,
        "a reused capability slot must name a fresh operation id"
    );

    // --- close rejects in-flight caps (must complete or be drained first) ----
    assert!(completion::close(asid, cap_reused).is_err()); // NotComplete
    completion::complete(asid, cap_reused, OpResult::Ok(0)).unwrap();
    completion::close(asid, cap_reused).unwrap(); // now it works

    // --- cancel: InFlight -> CancelPending -> Completed(Cancelled) -----------
    let cap2 = completion::submit(asid, OpCode::Write, Some(alloc::vec![1u8, 2, 3])).unwrap();
    assert_eq!(completion::cancel(asid, cap2).unwrap(), CancelState::CancelRequested);
    assert_eq!(completion::state_of(asid, cap2).unwrap(), OpStateKind::CancelPending);
    // Cancellation is idempotent while pending.
    assert_eq!(completion::cancel(asid, cap2).unwrap(), CancelState::CancelRequested);
    // Deferred reclaim: the kernel still owns the buffer while cancellation is
    // in flight — it may still touch it until the terminal completion.
    assert!(completion::holds_buffer(asid, cap2).unwrap());
    // A cancel-pending operation is not reclaimable yet.
    assert!(completion::close(asid, cap2).is_err());

    completion::complete(asid, cap2, OpResult::Ok(3)).unwrap();
    assert_eq!(completion::state_of(asid, cap2).unwrap(), OpStateKind::Completed);
    let done = completion::poll(asid, cap2).unwrap().expect("must be complete");
    assert!(matches!(done.result, OpResult::Cancelled));
    assert_eq!(done.buffer.as_deref(), Some(&[1u8, 2, 3][..]));
    completion::close(asid, cap2).unwrap();

    // --- submission backpressure (capacity 2) --------------------------------
    let _a = completion::submit(asid, OpCode::Nop, None).unwrap();
    let _b = completion::submit(asid, OpCode::Nop, None).unwrap();
    assert_eq!(completion::submit(asid, OpCode::Nop, None), Err(SubmitError::WouldBlock));

    completion::close_address_space(asid);

    // --- CQ overflow is retained in a kernel backlog, not lost --------------
    let cq_asid = 0xc0_ffee_01;
    completion::open_address_space_with_cq(cq_asid, 4, 2);
    let cap_a = completion::submit(cq_asid, OpCode::Nop, None).unwrap();
    let cap_b = completion::submit(cq_asid, OpCode::Nop, None).unwrap();

    completion::complete(cq_asid, cap_a, OpResult::Ok(10)).unwrap();
    completion::complete(cq_asid, cap_b, OpResult::Ok(20)).unwrap();

    let ring_ptr = completion::cq_ring_of(cq_asid).expect("CQ ring must exist");
    assert_eq!(unsafe { &*ring_ptr }.pending(), 1, "small CQ should hold the first entry");
    assert_eq!(unsafe { &*ring_ptr }.overflow, 1, "second entry should hit a full ring");

    let first = unsafe { &mut *ring_ptr }.read().expect("first CQ entry must be present");
    assert_eq!(first.cap, cap_a as u64);

    assert_eq!(
        completion::cq_pending(cq_asid),
        1,
        "cq_pending should flush the retained backlog entry"
    );
    let second = unsafe { &mut *ring_ptr }.read().expect("backlogged CQ entry must be posted");
    assert_eq!(second.cap, cap_b as u64);

    // --- a duplicate completion must not post a duplicate CQ entry -----------
    completion::complete(cq_asid, cap_a, OpResult::Ok(11)).unwrap();
    assert_eq!(
        completion::cq_pending(cq_asid),
        0,
        "idempotent re-completion must not produce a CQ entry"
    );

    // --- a cancelled operation's CQ entry carries the effective result -------
    let cap_c = completion::submit(cq_asid, OpCode::Nop, None).unwrap();
    assert_eq!(completion::cancel(cq_asid, cap_c).unwrap(), CancelState::CancelRequested);
    completion::complete(cq_asid, cap_c, OpResult::Ok(30)).unwrap();
    let third = unsafe { &mut *ring_ptr }.read().expect("cancelled CQ entry must be posted");
    assert_eq!(third.cap, cap_c as u64);
    assert_eq!(
        crate::completion::cq::i64_to_op_result(third.result),
        OpResult::Cancelled,
        "the CQ ring and the capability must agree on the effective result"
    );

    assert!(completion::poll(cq_asid, cap_a).unwrap().is_some());
    assert!(completion::poll(cq_asid, cap_b).unwrap().is_some());
    assert!(matches!(
        completion::poll(cq_asid, cap_c).unwrap().expect("cancelled op must drain").result,
        OpResult::Cancelled
    ));
    completion::close(cq_asid, cap_a).unwrap();
    completion::close(cq_asid, cap_b).unwrap();
    completion::close(cq_asid, cap_c).unwrap();
    completion::close_address_space(cq_asid);

    logln!("Completion-capability subsystem tests passed.");
}
