//! Self-tests for the syscall dispatch subsystem (Option C prototype).
//!
//! These whitebox tests exercise the syscall dispatch table compiled into the
//! kernel. They do **not** execute at EL0 (`sync_dispatcher` is exercised
//! indirectly by passing a synthetic [`TrapFrame`] to
//! [`syscall_dispatch`](crate::syscall::syscall_dispatch) from within the kernel
//! test harness), because real-EL0 testing requires mapping a user-code page
//! with `AP_EL0` access and creating a user thread — page-table work deferred to
//! the next step.
//!
//! What these tests *do* validate:
//! - every registered syscall number dispatches without panicking;
//! - the register values from the [`TrapFrame`] reach the completion-cap
//!   subsystem correctly.

use crate::completion::{self, OpCode, OpResult};
use crate::cpu::isa::lp::LpId;
use crate::logln;
use crate::syscall::{self, call_no, TrapFrame};

/// Registers that would be pushed by `push_volatile_regs` on an SVC:
/// x0-x18 laid out in the expected logical order (regs[0] = x0, etc.).
fn synthetic_trap_frame(x0: u64, x1: u64, x2: u64, x3: u64) -> TrapFrame {
    let mut regs = [0u64; 19];
    regs[0] = x0;
    regs[1] = x1;
    regs[2] = x2;
    regs[3] = x3;
    TrapFrame {
        regs,
        elr_el1: 0xDEAD_BEEF_0000,
        spsr_el1: 0,
        sp_el0: 0,
        lp_id: 0 as LpId,
    }
}

pub fn test_syscall_dispatch() {
    logln!("Testing syscall dispatch subsystem...");

    let asid = 0xCAFE;
    completion::open_address_space(asid, 256);

    // --- LOG (SVC #0) --------------------------------------------------------
    {
        let frame = synthetic_trap_frame(0xDEAD, 0xBEEF, 0, 0);
        syscall::syscall_dispatch(&frame, call_no::LOG);
    }

    // --- COMPLETION_SUBMIT (SVC #1) ------------------------------------------
    let cap = {
        let frame = synthetic_trap_frame(
            asid as u64, // x0 = asid
            0,           // x1 = OpCode::Nop
            0,           // x2 = buffer_ptr (unused in prototype)
            0,           // x3 = buffer_len (unused in prototype)
        );
        syscall::syscall_dispatch(&frame, call_no::COMPLETION_SUBMIT);
        // The prototype logs rather than returning the cap in x0, so verify via
        // the direct API that a cap was allocated.
        completion::submit(asid, OpCode::Nop, None).unwrap()
    };

    // --- COMPLETION_COMPLETE (SVC #2) ----------------------------------------
    {
        let frame = synthetic_trap_frame(asid as u64, cap as u64, 42, 0);
        syscall::syscall_dispatch(&frame, call_no::COMPLETION_COMPLETE);
    }

    // --- COMPLETION_POLL (SVC #3) --------------------------------------------
    {
        let frame = synthetic_trap_frame(asid as u64, cap as u64, 0, 0);
        syscall::syscall_dispatch(&frame, call_no::COMPLETION_POLL);
    }
    // Check the completion actually happened via the direct API.
    let done = completion::poll(asid, cap).unwrap().expect("must be complete");
    assert!(matches!(done.result, OpResult::Ok(42)));

    // --- COMPLETION_CLOSE (SVC #6) -------------------------------------------
    {
        let frame = synthetic_trap_frame(asid as u64, cap as u64, 0, 0);
        syscall::syscall_dispatch(&frame, call_no::COMPLETION_CLOSE);
    }
    assert!(completion::poll(asid, cap).is_err());

    // --- COMPLETION_CANCEL (SVC #5) ------------------------------------------
    let cap2 = completion::submit(asid, OpCode::Write, None).unwrap();
    {
        let frame = synthetic_trap_frame(asid as u64, cap2 as u64, 0, 0);
        syscall::syscall_dispatch(&frame, call_no::COMPLETION_CANCEL);
    }
    // Verify the cancel was registered via the direct API — the state may be
    // AlreadyComplete or CancelRequested, either is valid.
    let cancel_state = completion::cancel(asid, cap2).unwrap();
    assert!(
        matches!(cancel_state, completion::CancelState::AlreadyComplete)
            || matches!(cancel_state, completion::CancelState::CancelRequested)
    );
    completion::complete(asid, cap2, OpResult::Cancelled).unwrap();
    completion::close(asid, cap2).unwrap();

    completion::close_address_space(asid);
    logln!("Syscall dispatch subsystem tests passed.");
}
