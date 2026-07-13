//! Typed `no_std` wrappers for the CharlotteOS (catten) syscall ABI.
//!
//! Each function corresponds to exactly one `svc #N` instruction.
//!
//! ## Register convention
//!
//! The kernel derives the caller's ASID from the running thread context, so
//! `x0` is **not** used for an ASID parameter.  Arguments start at `x1`:
//!
//!   x0 — unused (kernel derives ASID)
//!   x1 — first argument
//!   x2 — second argument
//!   x3 — third argument
//!   x0 — return value (written back by the kernel)
//!   x1 — secondary return value (for MAILBOX_RECV, WAIT_TIMEOUT, etc.)
//!
//! # Example
//!
//! ```ignore
//! use catten_syscall::*;
//!
//! let cap = unsafe { submit(OpCode::Nop) };
//! let (ok, result) = unsafe { wait_timeout(cap, 5000) };
//! unsafe { thread_exit(); } // never returns
//! ```
#![no_std]

use core::arch::asm;

// ---- op codes --------------------------------------------------------------

/// Operation codes for COMPLETION_SUBMIT.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    Nop = 0,
    Read = 1,
    Write = 2,
}

// ---- raw svc primitives ----------------------------------------------------

/// Issue `svc #imm` with `x1=arg1, x2=arg2, x3=arg3`, return `x0`.
#[inline(always)]
unsafe fn svc3(imm: u16, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    let ret: u64;
    unsafe {
        match imm {
            1 => asm!("svc #1", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            2 => asm!("svc #2", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            3 => asm!("svc #3", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            4 => asm!("svc #4", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            5 => asm!("svc #5", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            6 => asm!("svc #6", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            7 => asm!("svc #7", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            8 => asm!("svc #8", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            9 => asm!("svc #9", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            10 => asm!("svc #10", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            12 => asm!("svc #12", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            13 => asm!("svc #13", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            14 => asm!("svc #14", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            15 => asm!("svc #15", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            16 => asm!("svc #16", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            17 => asm!("svc #17", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            _ => core::hint::unreachable_unchecked(),
        }
    }
    ret
}

/// Like [`svc3`] but also captures the x1 return value (for syscalls that
/// return a secondary value in x1, e.g. MAILBOX_RECV_CAP, WAIT_TIMEOUT).
#[inline(always)]
unsafe fn svc3_x1(imm: u16, arg1: u64, arg2: u64, arg3: u64) -> (u64, u64) {
    let ret: u64;
    let x1_out: u64;
    unsafe {
        match imm {
            10 => asm!("svc #10", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            11 => asm!("svc #11", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, in("x2") arg2, options(nostack, nomem, preserves_flags)),
            16 => asm!("svc #16", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            _ => core::hint::unreachable_unchecked(),
        }
    }
    (ret, x1_out)
}

// ---- public syscall wrappers ------------------------------------------------

/// Submit an async operation.  Returns a completion capability.
#[inline(always)]
pub unsafe fn submit(op: OpCode) -> u64 {
    unsafe { svc3(1, op as u64, 0, 0) }
}

/// Submit a Read with a user buffer.  `buf_ptr`/`buf_len` point to a
/// writable buffer in the caller's address space that the kernel fills.
/// Returns the capability.
#[inline(always)]
pub unsafe fn submit_read(buf_ptr: usize, buf_len: usize) -> u64 {
    unsafe { svc3(1, OpCode::Read as u64, buf_ptr as u64, buf_len as u64) }
}

/// Post a terminal result for a completion capability.
#[inline(always)]
pub unsafe fn complete(cap: u64, result_code: i64) {
    unsafe { svc3(2, cap, result_code as u64, 0); }
}

/// Non-blocking check: drain the completion if it is terminal.
#[inline(always)]
pub unsafe fn poll(cap: u64) {
    unsafe { svc3(3, cap, 0, 0); }
}

/// Block until the given capability reaches a terminal completion.
#[inline(always)]
pub unsafe fn wait(cap: u64) {
    unsafe { svc3(4, cap, 0, 0); }
}

/// Request cancellation of an in-flight capability.
#[inline(always)]
pub unsafe fn cancel(cap: u64) {
    unsafe { svc3(5, cap, 0, 0); }
}

/// Release a completed/drained capability slot.
#[inline(always)]
pub unsafe fn close(cap: u64) {
    unsafe { svc3(6, cap, 0, 0); }
}

/// Spawn a new EL0 thread pinned to `target_lp`, starting at `entry_vaddr`.
/// Returns the kernel-assigned thread id.
#[inline(always)]
pub unsafe fn spawn_thread(entry_vaddr: usize, target_lp: u32) -> u64 {
    unsafe { svc3(7, entry_vaddr as u64, target_lp as u64, 0) }
}

/// Terminate the calling EL0 thread.  Never returns.
#[inline(always)]
pub unsafe fn thread_exit() -> ! {
    unsafe { svc3(8, 0, 0, 0); }
    loop { core::hint::spin_loop(); }
}

/// Send a 64-bit message to the target LP's global mailbox.
/// Returns 0 on success, 1 on queue-full.
#[inline(always)]
pub unsafe fn mailbox_send_raw(target_lp: u32, message: u64) -> u64 {
    unsafe { svc3(9, target_lp as u64, message, 0) }
}

/// Receive a message from the calling LP's global mailbox.
/// Returns `(msg, 0)` on success, `(0, 1)` when empty.
#[inline(always)]
pub unsafe fn mailbox_recv_raw() -> (u64, u64) {
    unsafe { svc3_x1(10, 0, 0, 0) }
}

/// Block on a capability with a timeout in milliseconds.
/// Returns `(0, result_code)` on completion, `(1, _)` on timeout.
#[inline(always)]
pub unsafe fn wait_timeout(cap: u64, timeout_ms: u64) -> (u64, u64) {
    unsafe { svc3_x1(11, cap, timeout_ms, 0) }
}

/// Block until the calling LP's CQ ring has at least `min_complete` pending
/// entries.  Returns the current pending count.
#[inline(always)]
pub unsafe fn cq_wait(min_complete: u64) -> u64 {
    unsafe { svc3(12, min_complete, 0, 0) }
}

/// Open a sender capability targeting LP `target_lp`.  Returns the cap.
#[inline(always)]
pub unsafe fn mailbox_open_send(target_lp: u32) -> u64 {
    unsafe { svc3(13, target_lp as u64, 0, 0) }
}

/// Open a receiver capability for the calling LP.  Returns the cap.
#[inline(always)]
pub unsafe fn mailbox_open_recv() -> u64 {
    unsafe { svc3(14, 0, 0, 0) }
}

/// Send a message through a sender capability.
/// Returns 0 on success, 1 on queue-full, 2 on invalid cap.
#[inline(always)]
pub unsafe fn mailbox_send(cap: u64, message: u64) -> u64 {
    unsafe { svc3(15, cap, message, 0) }
}

/// Receive a message through a receiver capability.
/// Returns `(msg, 0)` on success, `(0, 1)` when empty, `(0, 2)` on invalid cap.
#[inline(always)]
pub unsafe fn mailbox_recv(cap: u64) -> (u64, u64) {
    unsafe { svc3_x1(16, cap, 0, 0) }
}

/// Close a mailbox capability.  Returns 0 on success, 1 on invalid cap.
#[inline(always)]
pub unsafe fn mailbox_close(cap: u64) -> u64 {
    unsafe { svc3(17, cap, 0, 0) }
}
