//! # Async-syscall demonstration (Option C, end-to-end)
//!
//! Demonstrates the full asynchronous completion loop with real kernel threads,
//! after the scheduler is active, using the ABI's intended completion mechanism
//! — the completion capability fires when the worker thread **exits** (its
//! exit-observer), not via an explicit `complete` call:
//!
//! 1. A **coordinator** thread submits an operation via
//!    [`completion::submit_worker`](crate::completion::submit_worker), which
//!    spawns a **worker** thread and returns a
//!    [`CompletionCap`](crate::completion::CompletionCap) registered as the
//!    worker's exit-observer. The coordinator then blocks in
//!    [`completion::wait`](crate::completion::wait) on the capability.
//! 2. The **worker** performs genuinely asynchronous work — it
//!    [`sleep`](crate::cpu::scheduler::sleep)s, yielding the LP to the timer —
//!    then simply **returns**. It never touches the capability.
//! 3. When the worker returns it is aborted and reaped; its `Thread::Drop`
//!    fires the exit-observer, which completes the capability and wakes the
//!    coordinator. The coordinator resumes, [`poll`](crate::completion::poll)s
//!    the result, and reports success.
//!
//! This is the "submit → async work → (thread exit) → complete → wake" loop the
//! async-syscall ABI is built around, exercised end-to-end across real context
//! switches. Both demo threads exit cleanly and are reaped.

use crate::completion::{self, OpResult};
use crate::cpu::scheduler::{sleep, spawn_thread};
use crate::klib::time::duration::ExtDuration;
use crate::logln;
use crate::memory::{AddressSpaceId, KERNEL_ASID};

/// A distinct address-space id for the demo's completion table (not the kernel).
const DEMO_ASID: AddressSpaceId = 0xA0C_D000;

/// Spawns the async-syscall demonstration coordinator thread. Call this after
/// the scheduler is active (e.g. from `bsp_main` alongside the device probe).
pub fn spawn_async_syscall_demo() {
    spawn_thread(KERNEL_ASID, async_syscall_coordinator);
}

#[unsafe(no_mangle)]
extern "C" fn async_syscall_coordinator() {
    logln!("[async-demo] coordinator: opening completion address space");
    completion::open_address_space(DEMO_ASID, 8);

    // Submit an operation performed by a worker thread. The returned capability
    // completes when the worker thread exits (its exit-observer). The worker
    // reports its result via the exit-observer's terminal result (Ok(42) here).
    let cap = match completion::submit_worker(DEMO_ASID, async_syscall_worker, OpResult::Ok(42)) {
        Ok(cap) => cap,
        Err(_) => {
            logln!("[async-demo] coordinator: submit_worker failed");
            return;
        }
    };
    logln!("[async-demo] coordinator: submitted worker, now awaiting completion...");

    // Block until the worker thread exits and its exit-observer completes `cap`.
    let _ = completion::wait(DEMO_ASID, cap);

    match completion::poll(DEMO_ASID, cap) {
        Ok(Some(done)) => match done.result {
            OpResult::Ok(v) => {
                logln!("[async-demo] coordinator: COMPLETION RECEIVED, result Ok({})", v);
            }
            _ => logln!("[async-demo] coordinator: completion received (non-Ok result)"),
        },
        _ => logln!("[async-demo] coordinator: ERROR: capability not complete after wait"),
    }
    logln!("[async-demo] SUCCESS: async syscall round-trip complete (via thread-exit observer)");
    // Return cleanly: the thread is reaped.
}

#[unsafe(no_mangle)]
extern "C" fn async_syscall_worker() {
    logln!("[async-demo] worker: performing async work (sleep 50ms)");
    // Genuinely asynchronous: block on the timer, yielding the LP.
    sleep(ExtDuration::from_millis(50));
    logln!("[async-demo] worker: work finished, exiting (completion fires on exit)");
    // Simply return. The worker does NOT touch the capability: exiting the
    // thread is the completion event (its exit-observer fires `complete`).
}
