//! Self-tests for shard-local state and typed mailbox dispatch (Option B).
//!
//! These tests validate the lock-free `ShardLocal<T>` owner-check / borrow-flag
//! discipline and the `ShardMailbox<M>` bounded send/receive contract on the
//! BSP (the only LP active at boot).

use alloc::sync::Arc;
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use crate::{
    cpu::{
        isa::lp::ops::get_lp_id,
        multiprocessor::{
            shard_mailbox,
            spin::shard_local::ShardLocal,
        },
    },
    logln,
};

pub fn test_shard_local() {
    logln!("Testing ShardLocal<T> lock-free discipline...");

    let sl: ShardLocal<u64> = ShardLocal::new(|| 42u64);

    // --- owner-LP access via `try_with` --------------------------------------
    let result = sl.try_with(|val| {
        assert_eq!(*val, 42);
        *val = 100;
        *val
    });
    assert_eq!(result, Ok(100));

    // --- value persists across accesses --------------------------------------
    let v = sl.try_with(|val| *val);
    assert_eq!(v, Ok(100));

    // --- re-entrant access is caught -----------------------------------------
    let flag = Arc::new(AtomicBool::new(false));
    let caught = flag.clone();
    let result = sl.try_with(|_val| {
        // Nested `try_with` must fail with AlreadyBorrowed.
        let inner = sl.try_with(|_| ());
        if inner.is_err() {
            caught.store(true, Ordering::SeqCst);
        }
    });
    assert!(result.is_ok(), "outer try_with must succeed");
    assert!(flag.load(Ordering::SeqCst), "nested try_with must be rejected");

    // --- `with()` panics on re-entrant access (tested via the Ok paths above,
    //     the panic path is verified by construction — the kernel aborts on
    //     panic so we don't deliberately panic in self-tests) ----------------

    logln!("ShardLocal<T> tests passed.");
}

pub fn test_shard_mailbox() {
    logln!("Testing typed ShardMailbox<M>...");

    let set: shard_mailbox::ShardMailboxSet<u64> =
        shard_mailbox::ShardMailboxSet::new(shard_mailbox::DEFAULT_CAPACITY);

    let lp = get_lp_id();

    // --- send to self + receive locally --------------------------------------
    let sender = set.sender_to(lp);
    let mut receiver = set.receiver_for(lp);

    // Initially empty.
    assert!(receiver.try_recv().is_none());

    // Send a message.
    assert!(sender.try_send(7).is_ok());

    // Drain the local IPI queue (which would fire on a real cross-LP send,
    // but for same-LP the message is already queued, no IPI needed to drain).
    // Confirm the receiver can pick it up.
    let msg = receiver.try_recv();
    assert_eq!(msg, Some(7));
    assert!(receiver.try_recv().is_none());

    // --- backpressure: fill the bounded queue --------------------------------
    let capacity = shard_mailbox::DEFAULT_CAPACITY;
    for i in 0..capacity {
        assert!(sender.try_send(i as u64).is_ok(), "send {i} must succeed");
    }
    // Queue should be full.
    let result = sender.try_send(999);
    assert!(result.is_err(), "send on full queue must return backpressure");
    assert_eq!(result, Err(999));

    // Drain everything.
    for i in 0..capacity {
        let msg = receiver.try_recv();
        assert_eq!(msg, Some(i as u64), "expected message {i}");
    }
    assert!(receiver.try_recv().is_none());

    // --- multiple senders (clone) --------------------------------------------
    let sender2 = sender.clone();
    assert_eq!(sender2.target_lp(), lp);
    assert!(sender2.try_send(42).is_ok());
    assert_eq!(receiver.try_recv(), Some(42));

    logln!("ShardMailbox<M> tests passed.");
}
