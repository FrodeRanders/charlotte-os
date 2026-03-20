unsafe extern "custom" {
    pub unsafe fn isr_lapic_timer();
}
core::arch::global_asm!(include_str!("context_switch.asm"));
