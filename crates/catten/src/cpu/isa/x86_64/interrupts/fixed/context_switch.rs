use crate::cpu::isa::lp::ops::halt;
use crate::cpu::scheduler::lp_schedulers::Status;
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::cpu::scheduler::threads::ThreadId;

unsafe extern "custom" {
    pub unsafe fn isr_context_switch();
    pub unsafe fn enter_init_thread_ctx();
}
core::arch::global_asm!(include_str!("context_switch.asm"));

#[unsafe(no_mangle)]
pub extern "C" fn set_next_thread() {
    let mut next_thread: ThreadId = ThreadId::default();
    if SYSTEM_SCHEDULER.read().get_local_scheduler().lock().next(&mut next_thread)
        == Status::Success
    {
        unsafe {
            core::arch::asm!(
                "wrfsbase {tid}",
                tid = in(reg) next_thread
            );
        }
    } else {
        halt!()
    }
}
