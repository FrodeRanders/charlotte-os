use crate::cpu::isa::{
    interface::interrupts::LocalIntCtlrIfce,
    interrupts::x2apic::X2Apic,
};

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
    let _ = crate::timers::try_process_local_events();
}
