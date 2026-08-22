#[unsafe(naked)]
/// Return directly from an x86 spurious-interrupt entry.
///
/// # Safety
///
/// This function may only be entered by the CPU through an interrupt gate
/// with a valid interrupt-return frame at the top of the current stack.
pub unsafe extern "custom" fn isr_spurious() {
    // Spurious interrupt handler does nothing
    core::arch::naked_asm! {
        "iretq"
    };
}
