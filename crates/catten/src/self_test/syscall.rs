//! Self-tests for the syscall dispatch subsystem.
//!
//! Exercises every dispatch route by calling syscall_dispatch directly with a
//! synthetic TrapFrame.

use crate::completion::{self, OpCode, OpResult};
use crate::cpu::isa::lp::LpId;
use crate::logln;
use crate::syscall::{self, call_no, TrapFrame};

fn synthetic_trap_frame(x0: u64, x1: u64, x2: u64, x3: u64) -> TrapFrame {
    let mut regs = [0u64; 19];
    regs[0] = x0;
    regs[1] = x1;
    regs[2] = x2;
    regs[3] = x3;
    TrapFrame { regs, elr_el1: 0xDEADBEEF0000, spsr_el1: 0, sp_el0: 0, lp_id: 0 as LpId }
}

pub fn test_syscall_dispatch() {
    logln!("Testing syscall dispatch subsystem...");
    let asid = 0xCAFE;
    completion::open_address_space(asid, 256);

    // LOG
    { let mut f = synthetic_trap_frame(0xDEAD, 0xBEEF, 0, 0); syscall::syscall_dispatch(&mut f, call_no::LOG); }
    // COMPLETION_SUBMIT
    let cap = completion::submit(asid, OpCode::Nop, None).unwrap();
    { let mut f = synthetic_trap_frame(asid as u64, cap as u64, 0, 0); syscall::syscall_dispatch(&mut f, call_no::COMPLETION_SUBMIT); }
    // COMPLETION_COMPLETE
    { let mut f = synthetic_trap_frame(asid as u64, cap as u64, 42, 0); syscall::syscall_dispatch(&mut f, call_no::COMPLETION_COMPLETE); }
    // COMPLETION_POLL
    { let mut f = synthetic_trap_frame(asid as u64, cap as u64, 0, 0); syscall::syscall_dispatch(&mut f, call_no::COMPLETION_POLL); }
    // Verify via direct API
    let done = completion::poll(asid, cap).unwrap();
    assert!(done.is_none(), "cap already drained by syscall dispatch");
    // CLOSE
    { let mut f = synthetic_trap_frame(asid as u64, cap as u64, 0, 0); syscall::syscall_dispatch(&mut f, call_no::COMPLETION_CLOSE); }
    // CANCEL (on a fresh cap)
    let cap2 = completion::submit(asid, OpCode::Write, None).unwrap();
    { let mut f = synthetic_trap_frame(asid as u64, cap2 as u64, 0, 0); syscall::syscall_dispatch(&mut f, call_no::COMPLETION_CANCEL); }
    completion::complete(asid, cap2, OpResult::Cancelled).unwrap();
    completion::close(asid, cap2).unwrap();

    completion::close_address_space(asid);
    logln!("Syscall dispatch subsystem tests passed.");
}
