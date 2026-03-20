use core::ffi::c_int;

use crate::common::time::duration::ExtDuration;
use crate::cpu::isa::interface::timers::LpTimerIfce;
use crate::cpu::isa::interrupts::x2apic::LAPICS;
use crate::cpu::isa::lp::ops::{await_interrupt, get_lp_id};
use crate::cpu::isa::lp::thread_context::ThreadContext;
use crate::cpu::isa::timers::LpTimer;
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::cpu::scheduler::threads::MASTER_THREAD_TABLE;
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
                let thread = tt_guard.get(tid).expect("Error getting thread from thread table.");
                let ctx_ptr = &raw const thread.context;
                return ctx_ptr as *mut ThreadContext;
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
pub extern "C" fn reset_lp_timer(micros: u64) -> core::ffi::c_int {
    if let Ok(mut lapic) = LAPICS.try_get_mut() {
        let lp_timer = &mut lapic.timer;
        lp_timer
            .set_duration(ExtDuration::from_micros(micros as u128))
            .expect("Error setting x2APIC timer duration.");
        lp_timer.reset().expect("Error resetting the LP timer.");
        0
    } else {
        -1
    }
}
