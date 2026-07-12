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

#[unsafe(no_mangle)]
extern "C" fn cross_lp_demo() {
    let lp_count = crate::cpu::multiprocessor::get_lp_count();
    if lp_count < 2 { logln!("[xLP] single LP, skipping"); return; }
    let my_lp = get_lp_id();
    logln!("[xLP] coordinator on LP{}: {} LPs detected", my_lp, lp_count);

    spawn_thread_on_lp(xlp_receiver_lp0, 0);
    spawn_thread_on_lp(xlp_receiver_lp1, 1);
    spawn_thread_on_lp(xlp_sender_lp0, 0);
    logln!("[xLP] coordinator: spawned LP0/LP1 receivers and LP0 sender");
}

#[unsafe(no_mangle)]
extern "C" fn xlp_sender_lp0() {
    let my_lp = get_lp_id();
    logln!("[xLP] sender on LP{}: waiting for receivers", my_lp);
    sleep(ExtDuration::from_millis(10));

    XLP_MAILBOX.sender_to(1).try_send(84).expect("xLP send to LP1 failed");
    logln!("[xLP] LP{}: sent 84 to LP1 via ShardMailbox + IPI", my_lp);

    XLP_MAILBOX.sender_to(0).try_send(21).expect("self-LP send failed");
    logln!("[xLP] LP{}: sent 21 to LP0 via ShardMailbox", my_lp);
}

#[unsafe(no_mangle)]
extern "C" fn xlp_receiver_lp0() {
    xlp_receiver_on_current_lp("LP0 local", 21);
}

#[unsafe(no_mangle)]
extern "C" fn xlp_receiver_lp1() {
    xlp_receiver_on_current_lp("LP1 remote", 84);
}

fn xlp_receiver_on_current_lp(label: &str, expected: u64) {
    let my_lp = get_lp_id();
    logln!("[xLP] receiver {} on LP{}: started", label, my_lp);
    let mut recv = XLP_MAILBOX.receiver_for_current_lp();
    for _ in 0..250 {
        if let Some(msg) = recv.try_recv() {
            if msg == expected {
                logln!("[xLP] receiver {} on LP{}: received {}", label, my_lp, msg);
            } else {
                logln!(
                    "[xLP] receiver {} on LP{}: received {}, expected {}",
                    label,
                    my_lp,
                    msg,
                    expected
                );
            }
            return;
        }
        sleep(ExtDuration::from_millis(1));
    }
    logln!("[xLP] receiver {} on LP{}: timed out", label, my_lp);
}
