use crate::cpu::isa::interface::interrupts::LocalIntCtlrIfce;
use crate::cpu::isa::interrupts::x2apic::X2Apic;
use crate::timers::TIMER_QUEUES;

unsafe extern "custom" {
    pub unsafe fn isr_lapic_timer();
}
core::arch::global_asm!(include_str!("timer.asm"));

#[unsafe(no_mangle)]
pub extern "C" fn signal_eoi() {
    X2Apic::signal_eoi();
}

#[unsafe(no_mangle)]
pub extern "C" fn process_events() {
    TIMER_QUEUES.try_get_mut().unwrap().process_events();
}
