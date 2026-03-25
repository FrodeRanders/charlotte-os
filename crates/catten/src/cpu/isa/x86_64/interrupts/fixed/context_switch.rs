use crate::common::time::duration::ExtDuration;
use crate::cpu::isa::interface::interrupts::LocalIntCtlrIfce;
use crate::cpu::isa::interface::timers::LpTimerIfce;
use crate::cpu::isa::interrupts::x2apic::{LAPICS, X2Apic};

unsafe extern "custom" {
    pub unsafe fn isr_lapic_timer();
}
core::arch::global_asm!(include_str!("context_switch.asm"));

#[unsafe(no_mangle)]
pub extern "C" fn signal_eoi() {
    X2Apic::signal_eoi();
}

#[unsafe(no_mangle)]
pub extern "C" fn reset_lp_timer() {
    unsafe {
        let mut lapic = LAPICS.try_get_mut().unwrap();
        let timer = &mut lapic.assume_init_mut().timer;
        timer
            .set_duration(ExtDuration::from_millis(10))
            .expect("Failed to set LAPIC timer duration.");
        timer.reset().expect("Failed to reset LAPIC timer.");
    }
}
