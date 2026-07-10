//! Self-test: integrated CQ ring + completion subsystem.
//!
//! Validates the full submit → complete → ring-entry cycle: open an AS with a
//! CQ ring, submit a capability, complete it (writes to the ring), and verify
//! the ring entry appears with correct values.

use crate::completion::{self, OpCode, OpResult};
use crate::logln;

pub fn test_cq_ring_in_completion() {
    logln!("Testing CQ ring integration with completion subsystem...");

    let asid = 0xC0FFEE;
    // Open an AS with a capability table (capacity 16) and a CQ ring (32 slots).
    completion::open_address_space_with_cq(asid, 16, 32);

    // --- submit + complete writes a ring entry --------------------------------
    let cap = completion::submit(asid, OpCode::Read, Some(alloc::vec![0u8; 4])).unwrap();

    // Before complete, the ring should be empty.
    assert_eq!(completion::cq_pending(asid), 0);

    // Complete: signals the observer AND writes to the CQ ring.
    completion::complete(asid, cap, OpResult::Ok(4)).unwrap();

    // Now the ring must show one pending entry.
    assert_eq!(completion::cq_pending(asid), 1);

    // Drain the ring entry and verify it.
    let ring_ptr = completion::cq_ring_of(asid)
        .expect("CQ ring must exist");
    let entry = unsafe { &mut *ring_ptr }.read().expect("first entry must be present");
    assert_eq!(entry.cap, cap as u64);
    assert_eq!(
        entry.result,
        crate::completion::cq::op_result_to_i64(OpResult::Ok(4))
    );

    assert_eq!(completion::cq_pending(asid), 0);

    // --- multiple completions → ordered entries -------------------------------
    let c1 = completion::submit(asid, OpCode::Nop, None).unwrap();
    let c2 = completion::submit(asid, OpCode::Write, None).unwrap();
    let c3 = completion::submit(asid, OpCode::Nop, None).unwrap();

    completion::complete(asid, c1, OpResult::Ok(1)).unwrap();
    completion::complete(asid, c2, OpResult::Cancelled).unwrap();
    completion::complete(asid, c3, OpResult::Err(2)).unwrap();

    assert_eq!(completion::cq_pending(asid), 3);

    let ring = unsafe { &mut *completion::cq_ring_of(asid).unwrap() };
    let e1 = ring.read().unwrap();
    assert_eq!(e1.cap, c1 as u64);
    assert_eq!(e1.result, crate::completion::cq::op_result_to_i64(OpResult::Ok(1)));

    let e2 = ring.read().unwrap();
    assert_eq!(e2.cap, c2 as u64);
    assert_eq!(
        e2.result,
        crate::completion::cq::op_result_to_i64(OpResult::Cancelled)
    );

    let e3 = ring.read().unwrap();
    assert_eq!(e3.cap, c3 as u64);
    assert_eq!(e3.result, crate::completion::cq::op_result_to_i64(OpResult::Err(2)));

    // Clean up — closes the AS and frees the CQ ring allocation.
    completion::close_address_space(asid);

    logln!("CQ ring integration tests passed.");
}
