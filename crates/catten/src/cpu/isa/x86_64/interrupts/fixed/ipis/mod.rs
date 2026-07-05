core::arch::global_asm!(include_str!("ipis.asm"));

unsafe extern "custom" {
    pub fn isr_asynchronous_ipi();
    pub fn isr_synchronous_ipi();
}

#[unsafe(no_mangle)]
pub extern "C" fn ih_asynchronous_ipi() {
    todo!()
}

#[unsafe(no_mangle)]
pub extern "C" fn ih_synchronous_ipi() {
    todo!()
}
