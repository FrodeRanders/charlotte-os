//! Typed `no_std` wrappers for the CharlotteOS (catten) syscall ABI.
//!
//! Each function corresponds to exactly one `svc #N` instruction.  The ASID
//! is **not** a parameter — the kernel derives it from the calling thread's
//! context, so userspace code never supplies or even knows its own ASID.
//!
//! # Example
//!
//! ```ignore
//! use catten_syscall::*;
//!
//! let cap = submit(OpCode::Nop, 0, 0);
//! let (ok, result) = wait_timeout(cap, 5000);
//! thread_exit(); // never returns
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

// ---- raw svc primitive -----------------------------------------------------

/// Raw syscall: `svc #imm` with 4 register arguments, returns x0.
///
/// The immediate must be a compile-time constant — it is encoded directly in
/// the SVC instruction, not passed through a register.
#[inline(always)]
unsafe fn svc4(imm: u16, x0: u64, x1: u64, x2: u64, x3: u64) -> u64 {
    let ret: u64;
    unsafe {
        match imm {
            0 => asm!("svc #0", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            1 => asm!("svc #1", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            2 => asm!("svc #2", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            3 => asm!("svc #3", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            4 => asm!("svc #4", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            5 => asm!("svc #5", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            6 => asm!("svc #6", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            7 => asm!("svc #7", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            8 => asm!("svc #8", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            9 => asm!("svc #9", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            10 => asm!("svc #10", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            11 => asm!("svc #11", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            12 => asm!("svc #12", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            13 => asm!("svc #13", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            14 => asm!("svc #14", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            15 => asm!("svc #15", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            16 => asm!("svc #16", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            17 => asm!("svc #17", inlateout("x0") x0 => ret, in("x1") x1, in("x2") x2, in("x3") x3, options(nostack, nomem, preserves_flags)),
            _ => core::hint::unreachable_unchecked(),
        }
    }
    ret
}

// ---- public syscall wrappers ------------------------------------------------

/// Log a message pointer + length pair.  Diagnostic only.
#[inline(always)]
pub unsafe fn log(ptr: *const u8, len: usize) {
    unsafe { svc4(0, ptr as u64, len as u64, 0, 0); }
}

/// Submit an async operation.  Returns a completion capability.
///
/// `op` selects Nop/Read/Write.  For Read operations, `buf_ptr`/`buf_len`
/// optionally point at a user buffer the kernel will write into.
#[inline(always)]
pub unsafe fn submit(op: OpCode, buf_ptr: usize, buf_len: usize) -> u64 {
    unsafe { svc4(1, op as u64, buf_ptr as u64, buf_len as u64, 0) }
}

/// Post a terminal result for a completion capability.
#[inline(always)]
pub unsafe fn complete(cap: u64, result_code: i64) {
    unsafe { svc4(2, cap, result_code as u64, 0, 0); }
}

/// Non-blocking check: drain the completion if it is terminal.
#[inline(always)]
pub unsafe fn poll(cap: u64) {
    unsafe { svc4(3, cap, 0, 0, 0); }
}

/// Block until the given capability reaches a terminal completion.
#[inline(always)]
pub unsafe fn wait(cap: u64) {
    unsafe { svc4(4, cap, 0, 0, 0); }
}

/// Request cancellation of an in-flight capability.
#[inline(always)]
pub unsafe fn cancel(cap: u64) {
    unsafe { svc4(5, cap, 0, 0, 0); }
}

/// Release a completed/drained capability slot.
#[inline(always)]
pub unsafe fn close(cap: u64) {
    unsafe { svc4(6, cap, 0, 0, 0); }
}

/// Spawn a new EL0 thread pinned to `target_lp`, starting at `entry_vaddr`.
/// Returns the kernel-assigned thread id.
#[inline(always)]
pub unsafe fn spawn_thread(entry_vaddr: usize, target_lp: u32) -> u64 {
    unsafe { svc4(7, entry_vaddr as u64, target_lp as u64, 0, 0) }
}

/// Terminate the calling EL0 thread.  Never returns.
#[inline(always)]
pub unsafe fn thread_exit() -> ! {
    unsafe { svc4(8, 0, 0, 0, 0); }
    loop { core::hint::spin_loop(); }
}

/// Send a 64-bit message to the target LP's global mailbox.
/// Returns 0 on success, 1 on queue-full.
#[inline(always)]
pub unsafe fn mailbox_send_raw(target_lp: u32, message: u64) -> u64 {
    unsafe { svc4(9, target_lp as u64, message, 0, 0) }
}

/// Receive a message from the calling LP's global mailbox.
/// Returns `(msg, 0)` on success, `(0, 1)` when empty.
#[inline(always)]
pub unsafe fn mailbox_recv_raw() -> (u64, u64) {
    let ret: u64;
    let status: u64;
    unsafe {
        asm!(
            "svc #10",
            inlateout("x0") 0u64 => ret,
            lateout("x1") status,
            options(nostack, nomem, preserves_flags),
        );
    }
    (ret, status)
}

/// Block on a capability with a timeout in milliseconds.
/// Returns `(0, result_code)` on completion, `(1, 0)` on timeout.
#[inline(always)]
pub unsafe fn wait_timeout(cap: u64, timeout_ms: u64) -> (u64, u64) {
    let ret: u64;
    let result: u64;
    unsafe {
        asm!(
            "svc #11",
            inlateout("x0") cap => ret,
            in("x1") timeout_ms,
            lateout("x2") result,
            options(nostack, nomem, preserves_flags),
        );
    }
    (ret, result)
}

/// Block until the calling LP's CQ ring has at least `min_complete` pending
/// entries.  Returns the current pending count.
#[inline(always)]
pub unsafe fn cq_wait(min_complete: u64) -> u64 {
    unsafe { svc4(12, min_complete, 0, 0, 0) }
}

/// Open a sender capability targeting LP `target_lp`.  Returns the cap.
#[inline(always)]
pub unsafe fn mailbox_open_send(target_lp: u32) -> u64 {
    unsafe { svc4(13, target_lp as u64, 0, 0, 0) }
}

/// Open a receiver capability for the calling LP.  Returns the cap.
#[inline(always)]
pub unsafe fn mailbox_open_recv() -> u64 {
    unsafe { svc4(14, 0, 0, 0, 0) }
}

/// Send a message through a sender capability.  Returns 0 on success,
/// 1 on queue-full, 2 on invalid cap.
#[inline(always)]
pub unsafe fn mailbox_send(cap: u64, message: u64) -> u64 {
    unsafe { svc4(15, cap, message, 0, 0) }
}

/// Receive a message through a receiver capability.
/// Returns `(msg, 0)` on success, `(0, 1)` when empty, `(0, 2)` on invalid cap.
#[inline(always)]
pub unsafe fn mailbox_recv(cap: u64) -> (u64, u64) {
    let ret: u64;
    let status: u64;
    unsafe {
        asm!(
            "svc #16",
            inlateout("x0") cap => ret,
            lateout("x1") status,
            options(nostack, nomem, preserves_flags),
        );
    }
    (ret, status)
}

/// Close a mailbox capability.  Returns 0 on success, 1 on invalid cap.
#[inline(always)]
pub unsafe fn mailbox_close(cap: u64) -> u64 {
    unsafe { svc4(17, cap, 0, 0, 0) }
}
