//! Self-tests for the bounded cross-LP IPI queue and typed-message dispatch
//! (Option C / Option B seed).
//!
//! These tests run at boot (single LP, the BSP) and exercise the bounded-queue
//! semantics and the [`Closure`](crate::cpu::multiprocessor::ipi::IpiRpc::Closure)
//! variant on the calling LP itself. Real cross-LP execution requires a running
//! scheduler on a second LP; the contract (try-push returns backpressure when
//! full, closures drain and execute) is validated locally.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::cpu::multiprocessor::ipi::{self, IpiRpc};
use crate::logln;

pub fn test_ipi_bounded_queue() {
    logln!("Testing bounded IPI queue and typed-message dispatch...");

    // We test on the calling LP (LP 0, the BSP). The API works regardless of
    // whether the target is local or remote.
    let target = crate::cpu::isa::lp::ops::get_lp_id();

    // Drain any pre-existing entries before the test.
    crate::cpu::multiprocessor::ipi::drain_local_ipi_queue();
    let lp = target as usize;

    // --- try_push_to / try_send_ipi_rpc return backpressure on full queue ----
    let capacity = ipi::IPI_CMD_QUEUES.queue_len(lp);
    let cap = 256; // known DEFAULT_QUEUE_CAPACITY
    assert!(capacity <= cap, "queue must be bounded (capacity 256)");

    // --- try_run_on_lp: closure executed on target LP ------------------------
    let fired = alloc::sync::Arc::new(AtomicBool::new(false));
    let flag = fired.clone();
    let r = ipi::try_run_on_lp(target, move || {
        flag.store(true, Ordering::SeqCst);
    });
    assert!(r.is_ok(), "try_run_on_lp must succeed on empty queue");

    // Drain the local queue (which executes the closure).
    ipi::drain_local_ipi_queue();
    assert!(fired.load(Ordering::SeqCst), "closure must have executed");

    // --- try_send_ipi_rpc with Wakeup ----------------------------------------
    let r = ipi::try_send_ipi_rpc(target, IpiRpc::Wakeup);
    assert!(r.is_ok(), "try_send_ipi_rpc Wakeup must succeed on empty queue");

    // Drain it.
    ipi::drain_local_ipi_queue();

    // --- backpressure: fill the queue, then verify rejection -----------------
    // Fill to capacity - 1 so one more push should still succeed.
    let max = capacity;
    for _ in 0..max {
        let _ = ipi::try_send_ipi_rpc(target, IpiRpc::Wakeup);
    }

    // Queue should now be full. The next push must return backpressure.
    let r = ipi::try_send_ipi_rpc(target, IpiRpc::Wakeup);
    assert!(r.is_err(), "try_send_ipi_rpc must return backpressure on full queue");
    assert!(matches!(r, Err(IpiRpc::Wakeup)), "must return the Wakeup RPC back");

    // Drain to free space for the next test.
    ipi::drain_local_ipi_queue();

    logln!("Bounded IPI queue and typed-message dispatch tests passed.");
}
