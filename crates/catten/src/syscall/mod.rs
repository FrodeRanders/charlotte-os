//! # Syscall Dispatch Subsystem
//!
//! This module defines the AArch64 syscall dispatch table and the per-ISA
//! [`TrapFrame`] type that the [`sync_dispatcher`] (when handling an SVC from a
//! lower EL) passes into the dispatch function. It also contains the public
//! `syscall_dispatch` entry point that [`sync_dispatcher`] calls after decoding
//! the exception class.
//!
//! At this prototype stage the syscalls mirror the completion-capability ABI
//! operations in [`crate::completion`], making only what already compiles
//! callable from a (future) user thread. The real user-register-to-semantic
//! mapping (which registers carry the buffer pointer, how buffer ownership
//! crosses the EL boundary, etc.) depends on the shared-memory CQ/SQ ring or a
//! copy-based IPC channel — neither exists yet. This module wires the dispatch
//! path itself, which is the prerequisite for everything else.
//!
//! ## Syscall number convention
//!
//! The AArch64 `SVC #imm` instruction encodes a 16-bit immediate in
//! `ESR_EL1[15:0]` (the ISS field for SVC). This kernel uses that immediate as
//! the syscall number.

use crate::cpu::isa::lp::LpId;

/// A snapshot of the volatile register set and architectural state at the moment
/// a synchronous exception was taken from a lower EL on AArch64.
///
/// `regs[0]` = x0, …, `regs[18]` = x18 as saved by `push_volatile_regs` on the
/// kernel stack. The caller ([`sync_dispatcher`]) populates `elr_el1`, `spsr_el1`,
/// and `sp_el0` from the saved system registers.
#[derive(Debug)]
pub struct TrapFrame {
    pub regs: [u64; 19],
    pub elr_el1: u64,
    pub spsr_el1: u64,
    pub sp_el0: u64,
    pub lp_id: LpId,
}

/// The upper bound on the SVC immediate we will try to dispatch.
pub const MAX_SYSCALL: u16 = 7;

/// Syscall numbers.
pub mod call_no {
    pub const LOG: u16 = 0;
    pub const COMPLETION_SUBMIT: u16 = 1;
    pub const COMPLETION_COMPLETE: u16 = 2;
    pub const COMPLETION_POLL: u16 = 3;
    pub const COMPLETION_WAIT: u16 = 4;
    pub const COMPLETION_CANCEL: u16 = 5;
    pub const COMPLETION_CLOSE: u16 = 6;
    /// Spawn a kernel thread pinned to a specific LP.
    pub const SPAWN_THREAD: u16 = 7;
}

/// Decode the exception class (EC) field from ESR_EL1 bits [31:26].
pub const fn ec_from_esr(esr: u64) -> u8 {
    ((esr >> 26) & 0x3F) as u8
}

/// Exception class for SVC from AArch64 state.
pub const EC_SVC_AARCH64: u8 = 0x15;

/// The single entry point from the ISA-specific [`sync_dispatcher`]. Panics on
/// an unknown syscall.
pub fn syscall_dispatch(frame: &mut TrapFrame, syscall_no: u16) {
    match syscall_no {
        call_no::LOG => sys_log(frame),
        call_no::COMPLETION_SUBMIT => sys_completion_submit(frame),
        call_no::COMPLETION_COMPLETE => sys_completion_complete(frame),
        call_no::COMPLETION_POLL => sys_completion_poll(frame),
        call_no::COMPLETION_WAIT => sys_completion_wait(frame),
        call_no::COMPLETION_CANCEL => sys_completion_cancel(frame),
        call_no::COMPLETION_CLOSE => sys_completion_close(frame),
        call_no::SPAWN_THREAD => sys_spawn_thread(frame),
        _ => panic!("Unknown syscall number: {}", syscall_no),
    }
}

// ---- individual syscall implementations ------------------------------------

fn sys_log(frame: &mut TrapFrame) {
    let _ptr = frame.regs[0] as *const u8;
    let _len = frame.regs[1] as usize;
    let lp = frame.lp_id;
    crate::early_logln!("[EL0 SYSCALL] LOG from userspace on LP {}", lp);
}

fn sys_completion_submit(frame: &mut TrapFrame) {
    let asid = frame.regs[0] as usize;
    let op_code = frame.regs[1];
    let op = match op_code {
        0 => crate::completion::OpCode::Nop,
        1 => crate::completion::OpCode::Read,
        2 => crate::completion::OpCode::Write,
        _ => panic!("Unknown op_code in syscall submit: {}", op_code),
    };
    match crate::completion::submit(asid, op, None) {
        Ok(cap) => frame.regs[0] = cap as u64,
        Err(_) => panic!("syscall completion submit failed"),
    }
}

fn sys_completion_complete(frame: &mut TrapFrame) {
    let asid = frame.regs[0] as usize;
    let cap = frame.regs[1] as usize;
    let result_code = frame.regs[2] as i64;
    let result = if result_code >= 0 {
        crate::completion::OpResult::Ok(result_code)
    } else {
        crate::completion::OpResult::Err(result_code as i32)
    };
    let _ = crate::completion::complete(asid, cap, result);
}

fn sys_completion_poll(frame: &mut TrapFrame) {
    let asid = frame.regs[0] as usize;
    let cap = frame.regs[1] as usize;
    match crate::completion::poll(asid, cap) {
        Ok(Some(_completed)) => {}
        Ok(None) => {}
        Err(_) => panic!("syscall completion poll failed: unknown cap {}", cap),
    }
}

fn sys_completion_wait(frame: &mut TrapFrame) {
    let asid = frame.regs[0] as usize;
    let cap = frame.regs[1] as usize;
    let _ = crate::completion::wait(asid, cap);
}

fn sys_completion_cancel(frame: &mut TrapFrame) {
    let asid = frame.regs[0] as usize;
    let cap = frame.regs[1] as usize;
    let _ = crate::completion::cancel(asid, cap);
}

fn sys_completion_close(frame: &mut TrapFrame) {
    let asid = frame.regs[0] as usize;
    let cap = frame.regs[1] as usize;
    let _ = crate::completion::close(asid, cap);
}

fn sys_spawn_thread(frame: &mut TrapFrame) {
    use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
    use crate::cpu::scheduler::threads::{MASTER_THREAD_TABLE, Thread};
    let asid = frame.regs[0] as crate::memory::AddressSpaceId;
    let entry_vaddr = frame.regs[1] as usize;
    let target_lp = frame.regs[2] as LpId;

    // A shard runs at EL0 in the *caller's* address space. `Thread::new` with a
    // non-kernel ASID builds a user thread context that drops to EL0 via
    // `user_trampoline`, loading the entry into ELR_EL1 and switching TTBR0 to
    // the caller's AS. The entry is therefore a virtual address in that AS and
    // must NOT be translated to a physical/HHDM pointer — doing so (and using
    // KERNEL_ASID) would run the target as an EL1 kernel thread.
    assert!(
        asid != crate::memory::KERNEL_ASID,
        "SPAWN_THREAD: refusing to spawn a shard into the kernel address space",
    );
    let entry_fn: extern "C" fn() =
        unsafe { core::mem::transmute::<usize, extern "C" fn()>(entry_vaddr) };

    // Create the thread and pin it directly to the requested LP. We must not go
    // through `scheduler::spawn_thread` (which submits to the least-loaded LP)
    // and then `submit_to_lp`: that would enqueue the same thread on two run
    // queues.
    let thread = Thread::new(asid, entry_fn);
    let tid = MASTER_THREAD_TABLE.write().add_element(thread);
    SYSTEM_SCHEDULER
        .read()
        .submit_to_lp(tid, target_lp)
        .expect("SPAWN_THREAD: failed to pin shard thread to target LP");
    // Return the thread id in x0.
    frame.regs[0] = tid as u64;
}
