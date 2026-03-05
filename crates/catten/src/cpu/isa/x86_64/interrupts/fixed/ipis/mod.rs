use crate::cpu::isa::interface::interrupts::LocalIntCtlrIfce;
use crate::cpu::isa::interrupts::LocalIntCtlr;

core::arch::global_asm!(include_str!("ipis.asm"));

unsafe extern "custom" {
    pub fn isr_interprocessor_interrupt();
}

#[unsafe(no_mangle)]
pub extern "C" fn handle_unicast_ipi() {
    LocalIntCtlr::signal_eoi();
}
