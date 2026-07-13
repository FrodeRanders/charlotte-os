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

    // --- complete posts the result, hands the buffer back on poll -----------
    completion::complete(asid, cap, OpResult::Ok(4)).unwrap();
    // Buffer stays with the kernel until poll() drains it.
    assert!(completion::holds_buffer(asid, cap).unwrap());

    let done = completion::poll(asid, cap).unwrap().expect("must be complete");
    assert!(!completion::holds_buffer(asid, cap).unwrap());
    assert!(matches!(done.result, OpResult::Ok(4)));
    assert_eq!(done.buffer.as_deref(), Some(&[0u8; 4][..]));

    // --- close frees the slot ------------------------------------------------
    completion::close(asid, cap).unwrap();
    assert!(completion::poll(asid, cap).is_err());

    // --- close rejects in-flight caps (must complete or be drained first) ----
    let cap_inflight = completion::submit(asid, OpCode::Nop, None).unwrap();
    assert!(completion::close(asid, cap_inflight).is_err()); // NotComplete
    completion::complete(asid, cap_inflight, OpResult::Ok(0)).unwrap();
    completion::close(asid, cap_inflight).unwrap(); // now it works

    // --- cancel retains the buffer until the terminal completion -------------
    let cap2 = completion::submit(asid, OpCode::Write, Some(alloc::vec![1u8, 2, 3])).unwrap();
    assert_eq!(completion::cancel(asid, cap2).unwrap(), CancelState::CancelRequested);
    // Deferred reclaim: the kernel still owns the buffer while cancellation is
    // in flight — it may still touch it until the terminal completion.
    assert!(completion::holds_buffer(asid, cap2).unwrap());

    completion::complete(asid, cap2, OpResult::Ok(3)).unwrap();
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

    assert!(completion::poll(cq_asid, cap_a).unwrap().is_some());
    assert!(completion::poll(cq_asid, cap_b).unwrap().is_some());
    completion::close(cq_asid, cap_a).unwrap();
    completion::close(cq_asid, cap_b).unwrap();
    completion::close_address_space(cq_asid);

    logln!("Completion-capability subsystem tests passed.");
}
