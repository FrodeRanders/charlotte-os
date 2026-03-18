use core::arch::asm;
use core::mem::transmute;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::common::time::duration::ExtDuration;
use crate::cpu::isa::interface::timers::LpTimerIfce;
use crate::cpu::isa::lp::ops::{await_interrupt, get_lp_id};
use crate::cpu::isa::lp::thread_context::ThreadContext;
use crate::cpu::isa::timers::LpTimer;
use crate::cpu::multiprocessor::get_lp_count;
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::cpu::scheduler::threads::{MASTER_THREAD_TABLE, THREAD_CTX_OFFSET, Thread, ThreadTable};
use crate::logln;

unsafe extern "custom" {
    pub unsafe fn isr_context_switch();
    pub unsafe fn isr_yield();
}
core::arch::global_asm!(include_str!("context_switch.asm"));

#[unsafe(no_mangle)]
pub extern "C" fn set_next_thread() -> *mut ThreadContext {
    loop {
        // Locks are held only for the duration of this block and are released before halting.
        let result = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().next();
        match result {
            Ok(tid) => {
                let lp_id = get_lp_id();
                logln!("LP {lp_id} Local Scheduler: Setting thread {tid} as the next to run.");
                let tt_guard = MASTER_THREAD_TABLE.read();
                if let Some(thread) = tt_guard.get(tid) {
                    return unsafe { thread.lock().get_ctx_ptr() };
                }
            }
            Err(_) => {
                logln!(
                    "LP{} Local Scheduler: No threads in the run queue. Halting LP.",
                    (get_lp_id())
                );
                // Place the LP into a halted state until a yield IPI breaks it out.
                await_interrupt!();
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn reset_lp_timer(micros: u64) {
    let lp_timer = LpTimer::get_local();
    let mut lpt_guard = lp_timer.lock();
    lpt_guard
        .set_duration(ExtDuration::from_micros(micros as u128))
        .expect("Error setting x2APIC timer duration.");
    lpt_guard.reset().expect("Error resetting the LP timer.");
}
