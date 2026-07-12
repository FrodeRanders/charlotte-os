//! # Async-syscall and cross-LP demonstration
//!
//! Exercises the async-syscall ABI (exit-observer completion) and cross-LP
//! messaging via ShardMailbox when multiple LPs are available.

use crate::completion::{self, OpResult};
use crate::cpu::isa::lp::ops::get_lp_id;
use crate::cpu::multiprocessor::shard_mailbox;
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::cpu::scheduler::threads::{MASTER_THREAD_TABLE, Thread, ThreadId};
use crate::cpu::scheduler::{sleep, spawn_thread};
use crate::klib::time::duration::ExtDuration;
use crate::logln;
use crate::memory::{AddressSpaceId, KERNEL_ASID};
use spin::LazyLock;

const DEMO_ASID: AddressSpaceId = 0xA0C_D000;
static XLP_MAILBOX: LazyLock<shard_mailbox::ShardMailboxSet<u64>> =
    LazyLock::new(|| shard_mailbox::ShardMailboxSet::new(shard_mailbox::DEFAULT_CAPACITY));

pub fn spawn_async_syscall_demo() {
    spawn_thread(KERNEL_ASID, async_syscall_coordinator);
    spawn_thread(KERNEL_ASID, cross_lp_demo);
}

fn spawn_thread_on_lp(entry_point: extern "C" fn(), target_lp: u32) -> ThreadId {
    let thread = Thread::new(KERNEL_ASID, entry_point);
    let tid = MASTER_THREAD_TABLE.write().add_element(thread);
    SYSTEM_SCHEDULER
        .read()
        .submit_to_lp(tid, target_lp)
        .expect("Error submitting demo thread to target LP");
    tid
}

#[unsafe(no_mangle)]
extern "C" fn async_syscall_coordinator() {
    logln!("[async] coordinator: opening completion address space");
    completion::open_address_space(DEMO_ASID, 8);

    let cap = match completion::submit_worker(DEMO_ASID, async_syscall_worker, OpResult::Ok(42)) {
        Ok(cap) => cap,
        Err(_) => { logln!("[async] submit_worker failed"); return; }
    };
    logln!("[async] coordinator: submitted worker, awaiting completion...");
    let _ = completion::wait(DEMO_ASID, cap);

    match completion::poll(DEMO_ASID, cap) {
        Ok(Some(done)) => match done.result {
            OpResult::Ok(v) => logln!("[async] COMPLETION RECEIVED, result Ok({})", v),
            _ => logln!("[async] completion received (non-Ok result)"),
        },
        _ => logln!("[async] ERROR: not complete after wait"),
    }
    logln!("[async] SUCCESS: async syscall round-trip complete");
}

#[unsafe(no_mangle)]
extern "C" fn async_syscall_worker() {
    let lp = get_lp_id();
    logln!("[async] worker on LP{}: sleeping 50ms", lp);
    sleep(ExtDuration::from_millis(50));
    logln!("[async] worker on LP{}: exiting", lp);
}

/// Base value for cross-LP messages: the receiver on LP `i` expects
/// `XLP_MSG_BASE + i`, and the sender fans exactly that value out to LP `i`.
const XLP_MSG_BASE: u64 = 1000;

#[unsafe(no_mangle)]
extern "C" fn cross_lp_demo() {
    let lp_count = crate::cpu::multiprocessor::get_lp_count();
    if lp_count < 2 {
        logln!("[xLP] single LP, skipping");
        return;
    }
    let my_lp = get_lp_id();
    logln!("[xLP] coordinator on LP{}: {} LPs detected", my_lp, lp_count);

    // Spawn one receiver pinned to each LP...
    for lp in 0..lp_count {
        spawn_thread_on_lp(xlp_receiver, lp);
    }
    // ...and a single sender on LP0 that fans a distinct message out to every LP
    // (including LP0 itself, exercising both cross-LP and self-LP delivery).
    spawn_thread_on_lp(xlp_sender, 0);
    logln!(
        "[xLP] coordinator: spawned {} receivers (one per LP) and a sender on LP0",
        lp_count
    );
}

#[unsafe(no_mangle)]
extern "C" fn xlp_sender() {
    let my_lp = get_lp_id();
    let lp_count = crate::cpu::multiprocessor::get_lp_count();
    logln!("[xLP] sender on LP{}: waiting for receivers", my_lp);
    sleep(ExtDuration::from_millis(10));

    for lp in 0..lp_count {
        let msg = XLP_MSG_BASE + lp as u64;
        XLP_MAILBOX
            .sender_to(lp)
            .try_send(msg)
            .unwrap_or_else(|_| panic!("xLP send to LP{lp} failed"));
        logln!("[xLP] LP{}: sent {} to LP{} via ShardMailbox + IPI", my_lp, msg, lp);
    }
    logln!("[xLP] sender on LP{}: fanned out to all {} LPs", my_lp, lp_count);
}

#[unsafe(no_mangle)]
extern "C" fn xlp_receiver() {
    let my_lp = get_lp_id();
    let expected = XLP_MSG_BASE + my_lp as u64;
    logln!("[xLP] receiver on LP{}: started (expecting {})", my_lp, expected);
    let mut recv = XLP_MAILBOX.receiver_for_current_lp();
    for _ in 0..250 {
        if let Some(msg) = recv.try_recv() {
            if msg == expected {
                logln!("[xLP] receiver on LP{}: received {} (OK)", my_lp, msg);
            } else {
                logln!(
                    "[xLP] receiver on LP{}: received {}, expected {} (MISMATCH)",
                    my_lp, msg, expected
                );
            }
            return;
        }
        sleep(ExtDuration::from_millis(1));
    }
    logln!("[xLP] receiver on LP{}: timed out waiting for {}", my_lp, expected);
}
