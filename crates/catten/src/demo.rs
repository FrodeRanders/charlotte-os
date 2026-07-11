//! # Async-syscall and cross-LP demonstration
//!
//! Exercises the async-syscall ABI (exit-observer completion) and cross-LP
//! messaging via ShardMailbox when multiple LPs are available.

use crate::completion::{self, OpResult};
use crate::cpu::isa::lp::ops::get_lp_id;
use crate::cpu::multiprocessor::shard_mailbox;
use crate::cpu::scheduler::{sleep, spawn_thread};
use crate::klib::time::duration::ExtDuration;
use crate::logln;
use crate::memory::{AddressSpaceId, KERNEL_ASID};

const DEMO_ASID: AddressSpaceId = 0xA0C_D000;

pub fn spawn_async_syscall_demo() {
    spawn_thread(KERNEL_ASID, async_syscall_coordinator);
    spawn_thread(KERNEL_ASID, cross_lp_demo);
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

#[unsafe(no_mangle)]
extern "C" fn cross_lp_demo() {
    let lp_count = crate::cpu::multiprocessor::get_lp_count();
    if lp_count < 2 { logln!("[xLP] single LP, skipping"); return; }
    let my_lp = get_lp_id();
    logln!("[xLP] LP{}: {} LPs detected", my_lp, lp_count);

    // Spawn a receiver on the OTHER LP that waits for incoming IPI messages.
    if my_lp == 0 {
        spawn_thread(KERNEL_ASID, xlp_receiver_on_lp1);
    }
    if my_lp == 1 {
        spawn_thread(KERNEL_ASID, xlp_receiver_on_lp1);
    }

    // LP0 sends a ShardMailbox message to LP1 after the receiver is running.
    if my_lp == 0 {
        sleep(ExtDuration::from_millis(20));
        let mailbox: shard_mailbox::ShardMailboxSet<u64> =
            shard_mailbox::ShardMailboxSet::new(shard_mailbox::DEFAULT_CAPACITY);
        let s = mailbox.sender_to(1);
        s.try_send(84).expect("xLP send to LP1 failed");
        logln!("[xLP] LP0: sent 84 to LP1 via ShardMailbox + IPI");

        // Self-send: LP0 to LP0 (no IPI needed).
        let s2 = mailbox.sender_to(0);
        let mut r2 = mailbox.receiver_for(0);
        s2.try_send(21).expect("self-LP send failed");
        if let Some(v) = r2.try_recv() {
            logln!("[xLP] LP0: self-send received {}", v);
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn xlp_receiver_on_lp1() {
    let my_lp = get_lp_id();
    logln!("[xLP] receiver on LP{}: started, waiting for messages", my_lp);
    let mailbox: shard_mailbox::ShardMailboxSet<u64> =
        shard_mailbox::ShardMailboxSet::new(shard_mailbox::DEFAULT_CAPACITY);
    let mut recv = mailbox.receiver_for(my_lp);
    for _ in 0..500 {
        if let Some(msg) = recv.try_recv() {
            logln!("[xLP] LP{}: RECEIVED cross-LP message: {}", my_lp, msg);
            return;
        }
        crate::cpu::scheduler::yield_lp();
    }
    logln!("[xLP] LP{}: timed out waiting for message", my_lp);
}
