//! Self-test for the shared-memory completion-queue ring (CQ).
//!
//! Validates the kernel-side producer logic: write completions, read/drain
//! them, overflow detection, and pending count correctness.

use crate::{
    completion::{
        OpResult,
        cq::{
            self,
            op_result_to_i64,
        },
    },
    logln,
};

pub fn test_cq_ring() {
    logln!("Testing completion-queue ring...");

    // Allocate a ring with 32 entry slots (fits in one page).
    let (_buf, ring_ptr) = cq::CompletionQueueRing::new_page(32);
    let ring = unsafe { &mut *ring_ptr };

    assert_eq!(ring.capacity, 32);
    assert_eq!(ring.pending(), 0);
    assert!(!ring.is_full());

    // --- write 3 entries, verify pending count ---------------------------------
    ring.write(1, OpResult::Ok(4));
    ring.write(2, OpResult::Ok(100));
    ring.write(3, OpResult::Cancelled);
    assert_eq!(ring.pending(), 3);

    // --- read back in insertion order ------------------------------------------
    let e0 = ring.read().expect("first entry must be present");
    assert_eq!(e0.cap, 1);
    assert_eq!(e0.result, op_result_to_i64(OpResult::Ok(4)));

    let e1 = ring.read().expect("second entry must be present");
    assert_eq!(e1.cap, 2);
    assert_eq!(e1.result, op_result_to_i64(OpResult::Ok(100)));

    let e2 = ring.read().expect("third entry must be present");
    assert_eq!(e2.cap, 3);
    assert_eq!(e2.result, op_result_to_i64(OpResult::Cancelled));

    assert_eq!(ring.pending(), 0);
    assert!(ring.read().is_none());

    // --- fill the ring to capacity - 1, then overflow --------------------------
    let cap = ring.capacity as usize;
    for i in 0..cap - 1 {
        assert!(ring.write((i + 100) as usize, OpResult::Ok(i as i64)));
    }
    assert!(ring.is_full());

    // Overflow: write one more, it must be dropped and overflow counter bumped.
    let dropped = !ring.write(999, OpResult::Ok(42));
    assert!(dropped, "write to full ring must return false");
    assert_eq!(ring.overflow, 1);

    // Drain everything, then the ring should be empty again.
    let mut count = 0;
    while ring.read().is_some() {
        count += 1;
    }
    assert_eq!(count, cap - 1, "must drain all entries written");
    assert_eq!(ring.pending(), 0);

    // --- verify error encoding round-trips -------------------------------------
    let mut ring2_buf;
    let ring2_ptr;
    {
        (ring2_buf, ring2_ptr) = cq::CompletionQueueRing::new_page(8);
    }
    let ring2 = unsafe { &mut *ring2_ptr };
    ring2.write(1, OpResult::Err(5));
    let e = ring2.read().unwrap();
    assert_eq!(cq::i64_to_op_result(e.result), OpResult::Err(5));
    drop(ring2_buf);

    logln!("Completion-queue ring tests passed.");
}
