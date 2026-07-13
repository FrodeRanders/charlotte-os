//! x86_64 SYSCALL entry trampoline.
//!
//! The `SYSCALL` instruction enters the kernel at the address configured in
//! `IA32_LSTAR` (set up by `init_lp_state()`). The asm trampoline saves the
//! user register context, calls the Rust handler, restores the context, and
//! returns to userspace via `SYSRETQ`.
//!
//! ## Limitations
//!
//! The trampoline currently saves only RCX/R11 (clobbered by SYSCALL) and the
//! original RAX syscall number.
//! A full implementation would save/restore all caller-saved registers and
//! build an architecture-specific TrapFrame for the dispatch layer. This is
//! sufficient for the prototype: it proves the SYSCALL entry path works.

use core::arch::global_asm;

global_asm!(
    ".global syscall_entry",
    "syscall_entry:",
    "swapgs",
    "mov gs:[0x8], rsp",
    "mov rsp, gs:[0x0]",
    "push r11",
    "push rcx",
    "push rax",
    "mov rdi, rax",
    "call {syscall_handler}",
    "add rsp, 8",
    "pop rcx",
    "pop r11",
    "mov rsp, gs:[0x8]",
    "swapgs",
    "sysretq",
    syscall_handler = sym crate::cpu::isa::x86_64::interrupts::syscall::syscall_handler,
);

/// The Rust-level syscall handler. Receives the syscall number in RDI
/// (SysV ABI first argument). Returns the result in RAX.
#[unsafe(no_mangle)]
pub extern "C" fn syscall_handler(syscall_no: u64) -> u64 {
    // For now, a minimal handler: echo the syscall number back as the return
    // value. Full dispatch with TrapFrame and register access is the next step.
    match syscall_no {
        0 => {
            // LOG
            0xC0FFEE
        }
        _ => {
            // Unknown syscall — return 0xDEAD.
            0xDEAD
        }
    }
}
