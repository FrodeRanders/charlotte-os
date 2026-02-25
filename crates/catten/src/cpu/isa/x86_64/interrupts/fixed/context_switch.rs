use crate::cpu::isa::lp::ops::halt;
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;

unsafe extern "custom" {
    pub unsafe fn isr_context_switch();
    pub unsafe fn enter_init_thread_ctx();
}
core::arch::global_asm!(include_str!("context_switch.asm"));

#[unsafe(no_mangle)]
pub extern "C" fn set_next_thread() {
    if let Ok(tid) = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().next() {
        unsafe {
            core::arch::asm!(
                "wrfsbase {tid}",
                tid = in(reg) tid
            );
        }
    } else {
        halt!()
    }
}
