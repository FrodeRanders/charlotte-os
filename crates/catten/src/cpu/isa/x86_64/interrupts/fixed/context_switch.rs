use crate::cpu::isa::lp::ops::{get_lp_id, halt};
use crate::cpu::isa::lp::thread_context::ThreadContext;
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::cpu::scheduler::threads::MASTER_THREAD_TABLE;
use crate::logln;

unsafe extern "custom" {
    pub unsafe fn isr_context_switch();
    pub unsafe fn enter_init_thread_ctx();
}
core::arch::global_asm!(include_str!("context_switch.asm"));

#[unsafe(no_mangle)]
pub extern "C" fn set_next_thread() -> *mut ThreadContext {
    if let Ok(tid) = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().next() {
        let lp_id = get_lp_id();
        logln!("LP{lp_id} Local Scheduler: Setting thread {tid} as the next to run.");
        let mut tt_guard = MASTER_THREAD_TABLE.write();
        let ctx_ptr = &raw mut tt_guard.get_mut(tid).as_mut().unwrap().context;
        logln!(
            "LP {lp_id} Local Scheduler: Locked the thread table and obtained the context \
             pointer={ctx_ptr:?} for thread {tid}"
        );
        ctx_ptr
    } else {
        logln!("LP{} Local Scheduler: No threads in the run queue. Halting LP.", (get_lp_id()));
        halt!()
    }
}
