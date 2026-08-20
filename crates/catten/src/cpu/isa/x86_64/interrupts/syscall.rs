//! x86_64 SYSCALL entry trampoline and Rust dispatch.
//!
//! The `SYSCALL` instruction enters the kernel at the address configured in
//! `IA32_LSTAR` (set up by [`init_lp_state`](crate::cpu::isa::x86_64::lp::ops::init_lp_state)).
//!
//! ## Register convention (mirrors the shared [`TrapFrame`] ABI)
//!
//! The kernel derives the caller's ASID from the per-LP [`CURRENT_LOGICAL_ASID`]
//! published by the context switch, so `rax` carries the syscall number and the
//! arguments start at `rdi`:
//!
//!   rax  — syscall number (on entry), return value (on exit)
//!   rdi  — first argument / secondary return value (`regs[1]`)
//!   rsi  — second argument (`regs[2]`)
//!   rdx  — third argument (`regs[3]`)
//!   r10  — fourth argument (`regs[4]`)
//!   r8   — fifth argument (`regs[5]`)
//!   r9   — sixth argument (`regs[6]`)
//!   r11  — extended return (`regs[7]`, e.g. the delivered memory cap)
//!   rcx  — extended return (`regs[8]`, e.g. the delivered connection cap)
//!   r12  — extended return (`regs[9]`, e.g. authenticated sender generation)
//!   r13  — extended return (`regs[10]`, e.g. authenticated sender principal)
//!   r14  — extended return (`regs[11]`, e.g. authenticated sender roles)
//!
//! `rbx`/`rbp` are callee-saved across the trap but are deliberately *not* used
//! as extended-return registers: LLVM reserves them, so they cannot appear as
//! `asm!` operands in the userspace syscall wrappers.
//!
//! The trampoline saves every general-purpose register, builds an `iretq` frame
//! for the return to ring 3 (the GDT ordering cannot satisfy `SYSRET`), and
//! calls the Rust handler. The handler builds a [`TrapFrame`], dispatches
//! through [`syscall::syscall_dispatch`], and writes the return registers back
//! before the trampoline restores them and `iretq`s to userspace.
//!
//! ## GS base
//!
//! The kernel accesses a per-LP [`PerCpuData`](crate::cpu::isa::x86_64::lp::ops::PerCpuData)
//! area (kernel stack top + a scratch slot) through GS. `IA32_KERNEL_GS_BASE`
//! is initialized for each LP, and the trampoline uses `swapgs` before touching
//! either the user stack or user-controlled memory.

use core::{
    arch::global_asm,
    sync::atomic::Ordering,
};

use crate::{
    cpu::isa::lp::ops::get_lp_id,
    early_logln,
    syscall::{
        self,
        MAX_SYSCALL,
        TrapFrame,
    },
};

global_asm!(
    ".global syscall_entry",
    "syscall_entry:",
    // SYSCALL leaves RSP at a user-controlled address. Switch GS and stacks
    // before the first write: writing register saves through the user RSP at
    // CPL0 would otherwise give userspace an arbitrary kernel-memory write.
    "swapgs",
    "mov gs:[0x8], rsp",        // save true user rsp in the scratch slot
    "mov rsp, gs:[0x0]",        // switch to this thread's kernel stack
    // Build the iretq frame (pushed first => highest addresses). The RSP field
    // is the true user stack pointer, captured above.
    "push 0x23",                // SS = user data selector
    "push qword ptr gs:[0x8]",  // RSP = user rsp
    "push r11",                 // RFLAGS = user rflags
    "push 0x1b",                // CS = user code selector
    "push rcx",                 // RIP = user rip
    // Build the TrapFrame GPR slots: regs[18]..regs[0] on the kernel stack.
    "push 0",                   // regs[18]
    "push 0",                   // regs[17]
    "push 0",                   // regs[16]
    "push 0",                   // regs[15]
    "push rbp",                 // regs[14]
    "push rbx",                 // regs[13]
    "push r15",                 // regs[12]
    "push r14",                 // regs[11]
    "push r13",                 // regs[10]
    "push r12",                 // regs[9]
    "push rcx",                 // regs[8]
    "push r11",                 // regs[7]
    "push r9",                  // regs[6]
    "push r8",                  // regs[5]
    "push r10",                 // regs[4]
    "push rdx",                 // regs[3]
    "push rsi",                 // regs[2]
    "push rdi",                 // regs[1]
    "push rax",                 // regs[0] = syscall number
    "mov rdi, rsp",             // frame_base = &regs[0]
    // Entry state now lives entirely on the trusted kernel stack. Keep
    // preemption and TLB-shootdown IPIs live while Rust dispatches or waits
    // for a contended lock.
    "sti",
    "call {syscall_entry_handler}",
    // The GS swap and privilege return must be atomic with respect to IRQs.
    "cli",
    // Restore the return registers.
    "pop rax",                  // regs[0]
    "pop rdi",                  // regs[1]
    "pop rsi",                  // regs[2]
    "pop rdx",                  // regs[3]
    "pop r10",                  // regs[4]
    "pop r8",                   // regs[5]
    "pop r9",                   // regs[6]
    "pop r11",                  // regs[7]
    "pop rcx",                  // regs[8]
    "pop r12",                  // regs[9]
    "pop r13",                  // regs[10]
    "pop r14",                  // regs[11]
    "pop r15",                  // regs[12]
    "pop rbx",                  // regs[13]
    "pop rbp",                  // regs[14]
    "add rsp, 32",              // skip regs[15..18]
    // Restore the userspace GS base and return through the ring-3 frame.
    "swapgs",
    "iretq",
    syscall_entry_handler = sym crate::cpu::isa::x86_64::interrupts::syscall::syscall_entry_handler,
);

/// The Rust-level SYSCALL handler. `frame_base` points at `regs[0]` of a
/// 19-slot register frame laid out as described in the module docs. Reads the
/// syscall number from `regs[0]` (RAX), dispatches through the shared syscall
/// table, and writes the return registers back into the frame.
#[unsafe(no_mangle)]
pub extern "C" fn syscall_entry_handler(frame_base: *mut u64) {
    let mut frame = TrapFrame {
        regs: [0u64; 19],
        elr_el1: 0,
        spsr_el1: 0,
        sp_el0: 0,
        lp_id: get_lp_id(),
        asid: crate::memory::KERNEL_ASID,
    };
    unsafe {
        for (index, reg) in frame.regs.iter_mut().enumerate() {
            *reg = frame_base.add(index).read_volatile();
        }
    }

    let syscall_no = frame.regs[0] as u16;
    frame.asid = crate::cpu::isa::x86_64::memory::paging::CURRENT_LOGICAL_ASID
        [get_lp_id() as usize]
        .load(Ordering::Acquire);

    if syscall_no > MAX_SYSCALL {
        early_logln!("FATAL EL0 UNKNOWN SYSCALL: ASID={} N={}", frame.asid, syscall_no);
        crate::cpu::scheduler::abort_address_space(frame.asid);
    }

    syscall::syscall_dispatch(&mut frame, syscall_no);

    unsafe {
        for (index, reg) in frame.regs.iter().enumerate() {
            frame_base.add(index).write_volatile(*reg);
        }
    }
}
