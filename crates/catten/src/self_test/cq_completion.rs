//! Self-test: integrated CQ ring + completion subsystem.
//!
//! Validates the full submit → complete → ring-entry cycle: open an AS with a
//! CQ ring, submit a capability, complete it (writes to the ring), and verify
//! the ring entry appears with correct values.

use crate::{
    completion::{
        self,
        OpCode,
        OpResult,
    },
    logln,
};

pub fn test_cq_ring_in_completion() {
    logln!("Testing CQ ring integration with completion subsystem...");

    let asid = 0xc0ffee;
    // Open an AS with a capability table (capacity 16) and a CQ ring (32 slots).
    completion::open_address_space_with_cq(asid, 16, 32).expect("CQ setup failed");

    // --- submit + complete writes a ring entry --------------------------------
    let cap = completion::submit(asid, OpCode::Read, Some(alloc::vec![0u8; 4])).unwrap();

    // Before complete, the ring should be empty.
    assert_eq!(completion::cq_pending(asid, 0), 0);

    // Complete: signals the observer AND writes to the CQ ring.
    completion::complete(asid, cap, OpResult::Ok(4)).unwrap();

    // Now the ring must show one pending entry.
    assert_eq!(completion::cq_pending(asid, 0), 1);

    // Drain the ring entry and verify it.
    let ring_ptr = unsafe { completion::cq_ring_of(asid, 0) }.expect("CQ ring must exist");
    // SAFETY: this private fixture retains the ring and is its sole consumer.
    let entry = unsafe { catten_syscall::cq_read(ring_ptr as usize, 32) }
        .expect("first entry must be present");
    assert_eq!(entry.cookie, cap as u64);
    assert_eq!(entry.result, crate::completion::cq::op_result_to_i64(OpResult::Ok(4)));

    assert_eq!(completion::cq_pending(asid, 0), 0);

    // --- multiple completions → ordered entries -------------------------------
    let c1 = completion::submit(asid, OpCode::Nop, None).unwrap();
    let c2 = completion::submit(asid, OpCode::Write, None).unwrap();
    let c3 = completion::submit(asid, OpCode::Nop, None).unwrap();

    completion::complete(asid, c1, OpResult::Ok(1)).unwrap();
    completion::complete(asid, c2, OpResult::Cancelled).unwrap();
    completion::complete(asid, c3, OpResult::Err(2)).unwrap();

    assert_eq!(completion::cq_pending(asid, 0), 3);

    let ring = unsafe { completion::cq_ring_of(asid, 0).unwrap() };
    // SAFETY: the retained fixture mapping still has this sole consumer.
    let e1 = unsafe { catten_syscall::cq_read(ring as usize, 32).unwrap() };
    assert_eq!(e1.cookie, c1);
    assert_eq!(e1.result, crate::completion::cq::op_result_to_i64(OpResult::Ok(1)));

    // SAFETY: the retained fixture mapping still has this sole consumer.
    let e2 = unsafe { catten_syscall::cq_read(ring as usize, 32).unwrap() };
    assert_eq!(e2.cookie, c2);
    assert_eq!(e2.result, crate::completion::cq::op_result_to_i64(OpResult::Cancelled));

    // SAFETY: the retained fixture mapping still has this sole consumer.
    let e3 = unsafe { catten_syscall::cq_read(ring as usize, 32).unwrap() };
    assert_eq!(e3.cookie, c3);
    assert_eq!(e3.result, crate::completion::cq::op_result_to_i64(OpResult::Err(2)));

    // Exercise repeated slot reuse through the production CqState producer
    // and the actual SDK consumer, not a second kernel-only decoder.
    for value in 0..128u64 {
        let cap = completion::submit(asid, OpCode::Nop, None).unwrap();
        let operation = completion::operation_id(asid, cap).unwrap();
        completion::complete(asid, cap, OpResult::Ok(value as i64)).unwrap();
        // SAFETY: fixture owns the live full-page mapping and its sole reader.
        let entry = unsafe { catten_syscall::cq_read(ring as usize, 32) }.unwrap();
        assert_eq!(
            (entry.operation, entry.cookie, entry.status, entry.flags, entry.result),
            (operation, cap, 0, 0, value as i64)
        );
        assert_eq!(completion::cq_pending(asid, 0), 0);
        // CQ reads do not observe or consume the operation capability.
        assert!(completion::poll(asid, cap).unwrap().is_some());
        completion::close(asid, cap).unwrap();
    }
    logln!("[CQ contract] production producer / SDK consumer: 128 wrapped records verified");

    // Clean up — closes the AS and frees the CQ ring allocation.
    completion::close_address_space(asid);

    logln!("CQ ring integration tests passed.");
}
