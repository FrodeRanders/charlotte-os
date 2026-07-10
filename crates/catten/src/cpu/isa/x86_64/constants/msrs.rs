//! # x86-64 Model Specific Registers (MSRs)
//!
//! Make sure to check any necessary CPUID features before using these MSRs as not all of them are
//! architectural.

#[inline(always)]
pub unsafe fn read(msr: u32) -> u64 {
    //! Read from an x86-64 model specific register
    let low: u32;
    let high: u32;

    unsafe {
        core::arch::asm!(
            "rdmsr",
            out("eax") low,
            out("edx") high,
            in("ecx") msr,
            options(nomem, nostack)
        );
    }

    ((high as u64) << 32) | (low as u64)
}

#[inline(always)]
pub unsafe fn write(msr: u32, value: u64) {
    //! Write to an x86-64 model specific register
    let low = value as u32;
    let high = (value >> 32) as u32;

    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("eax") low,
            in("edx") high,
            in("ecx") msr,
            options(nomem, nostack)
        );
    }
}

pub mod x2apic {
    //! # x2APIC MSRs
    //! Ref: AMD APM 16.11.1 and Intel SDM Vol.3 12.12.1.2
    pub const ID_REG: u32 = 0x802;
    pub const EOI_REGISTER: u32 = 0x80b;
    pub const LOGICAL_DEST_REG: u32 = 0x80d;
    pub const SPURIOUS_INTERRUPT_VECTOR_REG: u32 = 0x80f;
    pub const INTERRUPT_COMMAND_REGISTER: u32 = 0x830;
    pub const TIMER_LVTR: u32 = 0x832;
    pub const TIMER_INITIAL_COUNT: u32 = 0x838;
    pub const TIMER_CURRENT_COUNT: u32 = 0x839;
    pub const TIMER_DIVIDE_CONFIGURATION: u32 = 0x83e;
}

/// # TSC_AUX MSR
pub const TSC_AUX: u32 = 0xc000_0103;

// ---- SYSCALL / SYSRET MSRs -----------------------------------------------

/// Extended Feature Enable Register. Bit 0 (SCE) must be set for SYSCALL.
pub const EFER: u32 = 0xC000_0080;
pub const EFER_SCE: u64 = 1;

/// Bits 47:32 = SYSCALL CS selector, bits 63:48 = SYSRET CS selector.
pub const STAR: u32 = 0xC000_0081;
/// RIP loaded on SYSCALL (long mode handler entry point).
pub const LSTAR: u32 = 0xC000_0082;
/// RFLAGS mask applied on SYSCALL entry (bits set here are cleared).
pub const SFMASK: u32 = 0xC000_0084;

/// Convenience: enable SYSCALL by setting EFER.SCE.
pub unsafe fn enable_syscall() {
    let efer_val = read(EFER);
    write(EFER, efer_val | EFER_SCE);
}

/// Configure SYSCALL entry point. `handler` must be the address of the
/// assembly trampoline that saves registers and calls into Rust.
pub unsafe fn setup_syscall(handler_addr: u64) {
    // IA32_STAR: user segments are at GDT indices 3 (user data) and 4 (user
    // code, 64-bit). SYSRET returns to CPL3 with the selectors derived from
    // STAR[63:48] (user CS) and STAR[63:48] + 8 (user SS).
    //
    // Kernel segments: GDT index 1 = kernel code (ring 0), index 2 = kernel data.
    // SYSCALL loads CS = STAR[47:32], SS = STAR[47:32] + 8.
    let star_val: u64 = (0x0010u64 << 48)  // SYSRET CS = GDT[4] = 0x20, but with RPL bits: index 4, RPL 0 = 0x20, and +8 = 0x28 for SS
                         | (0x0008u64 << 32); // SYSCALL  CS = GDT[1] = 0x08, SS = 0x10
    write(STAR, star_val);
    write(LSTAR, handler_addr);
    // Clear interrupt flag on entry (mask interrupts during syscall dispatch).
    write(SFMASK, 1 << 9); // RFLAGS.IF = bit 9
    enable_syscall();
}
