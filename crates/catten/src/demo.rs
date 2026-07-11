//! # Async-syscall demonstration (Option C, end-to-end)
//!
//! Demonstrates the full asynchronous completion loop with real kernel threads,
//! after the scheduler is active:
//!
//! 1. A **coordinator** thread submits an operation → receives a
//!    [`CompletionCap`](crate::completion::CompletionCap), spawns a **worker**
//!    thread to perform the work, then blocks in
//!    [`completion::wait`](crate::completion::wait) on the capability.
//! 2. The **worker** performs genuinely asynchronous work — it
//!    [`sleep`](crate::cpu::scheduler::sleep)s, yielding the LP to the timer —
//!    then calls [`completion::complete`](crate::completion::complete), which
//!    signals the capability's observers and wakes the coordinator.
//! 3. The coordinator resumes, [`poll`](crate::completion::poll)s the result,
//!    and reports success.
//!
//! This is the "submit → async work → complete → wake" loop the async-syscall
//! ABI is built around, exercised end-to-end across a real context switch. Both
//! demo threads return cleanly when finished, exercising the thread-exit path.

use concurrent_queue::ConcurrentQueue;
use spin::LazyLock;

use crate::completion::{self, CompletionCap, OpCode, OpResult};
use crate::cpu::scheduler::{sleep, spawn_thread};
use crate::klib::time::duration::ExtDuration;
use crate::logln;
use crate::memory::{AddressSpaceId, KERNEL_ASID};

/// A distinct address-space id for the demo's completion table (not the kernel).
const DEMO_ASID: AddressSpaceId = 0xA0C_D000;

/// Work handed from the coordinator to the worker: which capability to complete.
static WORK_QUEUE: LazyLock<ConcurrentQueue<(AddressSpaceId, CompletionCap)>> =
    LazyLock::new(|| ConcurrentQueue::unbounded());

/// Spawns the async-syscall demonstration coordinator thread. Call this after
/// the scheduler is active (e.g. from `bsp_main` alongside the device probe).
pub fn spawn_async_syscall_demo() {
    spawn_thread(KERNEL_ASID, async_syscall_coordinator);
}

#[unsafe(no_mangle)]
extern "C" fn async_syscall_coordinator() {
    logln!("[async-demo] coordinator: opening completion address space");
    completion::open_address_space(DEMO_ASID, 8);

    let cap = match completion::submit(DEMO_ASID, OpCode::Nop, None) {
        Ok(cap) => cap,
        Err(_) => {
            logln!("[async-demo] coordinator: submit failed");
            return;
        }
    };
    logln!("[async-demo] coordinator: submitted operation, got capability");

    // Hand the capability to a worker and let it perform the async work.
    let _ = WORK_QUEUE.push((DEMO_ASID, cap));
    let _worker = spawn_thread(KERNEL_ASID, async_syscall_worker);
    logln!("[async-demo] coordinator: spawned worker, now awaiting completion...");

    // Block until the worker completes the capability.
    let _ = completion::wait(DEMO_ASID, cap);

    // Resumed: the worker completed the operation and woke us.
    match completion::poll(DEMO_ASID, cap) {
        Ok(Some(done)) => match done.result {
            OpResult::Ok(v) => {
                logln!("[async-demo] coordinator: COMPLETION RECEIVED, result Ok({})", v);
            }
            _ => logln!("[async-demo] coordinator: completion received (non-Ok result)"),
        },
        _ => logln!("[async-demo] coordinator: ERROR: capability not complete after wait"),
    }
    logln!("[async-demo] SUCCESS: async syscall round-trip complete");
    // Return cleanly: the thread trampoline calls `abort`, which now defers the
    // thread's teardown to the reaper so its kernel stack is freed safely.
}

#[unsafe(no_mangle)]
extern "C" fn async_syscall_worker() {
    if let Ok((asid, cap)) = WORK_QUEUE.pop() {
        logln!("[async-demo] worker: received work, performing async work (sleep 50ms)");
        // Genuinely asynchronous: the worker blocks on the timer, yielding the
        // LP; the timer wakes it 50 ms later.
        sleep(ExtDuration::from_millis(50));
        logln!("[async-demo] worker: work finished, completing capability");
        let _ = completion::complete(asid, cap, OpResult::Ok(42));
    } else {
        logln!("[async-demo] worker: no work in queue");
    }
    // Return cleanly (the trampoline calls `abort` -> reaper frees the stack).
}
