use crate::cpu::isa::{
    interface::interrupts::LocalIntCtlrIfce,
    interrupts::x2apic::X2Apic,
};

unsafe extern "custom" {
    pub unsafe fn isr_lapic_timer();
}
core::arch::global_asm!(
    include_str!("../fp_state.asm"),
    include_str!("timer.asm"),
    ".purgem catten_save_fp",
    ".purgem catten_restore_fp",
);

#[unsafe(no_mangle)]
pub extern "C" fn signal_eoi() {
    X2Apic::signal_eoi();
}

#[unsafe(no_mangle)]
pub extern "C" fn process_events() {
    let _ = crate::timers::try_process_local_events();
    #[cfg(feature = "kernel_tests")]
    {
        crate::self_test::clobber_x86_fp_state();
        crate::self_test::note_x86_timer_fp_clobber();
    }
}
