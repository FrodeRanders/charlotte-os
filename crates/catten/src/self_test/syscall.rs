//! Self-tests for the syscall dispatch subsystem.
//!
//! Exercises every dispatch route by calling syscall_dispatch directly with a
//! synthetic TrapFrame.

use crate::{
    completion::{
        self,
        OpCode,
        OpResult,
    },
    cpu::{
        isa::lp::LpId,
        multiprocessor::get_lp_count,
    },
    logln,
    syscall::{
        self,
        TrapFrame,
        call_no,
    },
};

fn synthetic_trap_frame(x0: u64, x1: u64, x2: u64, x3: u64) -> TrapFrame {
    let mut regs = [0u64; 19];
    regs[0] = x0;
    regs[1] = x1;
    regs[2] = x2;
    regs[3] = x3;
    TrapFrame {
        regs,
        elr_el1: 0xdeadbeef0000,
        spsr_el1: 0,
        sp_el0: 0,
        lp_id: 0 as LpId,
        asid: crate::memory::KERNEL_ASID,
    }
}

pub fn test_syscall_dispatch() {
    logln!("Testing syscall dispatch subsystem...");
    let asid = 0xcafe;
    completion::open_address_space(asid, 256);

    // LOG
    {
        let mut f = synthetic_trap_frame(0xdead, 0xbeef, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::LOG);
    }
    // COMPLETION_SUBMIT
    let cap = completion::submit(asid, OpCode::Nop, None).unwrap();
    {
        let mut f = synthetic_trap_frame(asid as u64, cap as u64, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_SUBMIT);
    }
    // COMPLETION_COMPLETE
    {
        let mut f = synthetic_trap_frame(asid as u64, cap as u64, 42, 0);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_COMPLETE);
    }
    // COMPLETION_POLL
    {
        let mut f = synthetic_trap_frame(asid as u64, cap as u64, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_POLL);
        assert_eq!(f.regs[0], 0, "poll should report completed");
        assert_eq!(f.regs[1] as i64, 42, "poll should return result code");
        assert_eq!(f.regs[2], 0, "poll should report no returned buffer");
    }
    // Verify via direct API
    let done = completion::poll(asid, cap).unwrap();
    assert!(done.is_none(), "cap already drained by syscall dispatch");
    // CLOSE
    {
        let mut f = synthetic_trap_frame(asid as u64, cap as u64, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_CLOSE);
    }
    // CANCEL (on a fresh cap)
    let cap2 = completion::submit(asid, OpCode::Write, None).unwrap();
    {
        let mut f = synthetic_trap_frame(asid as u64, cap2 as u64, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_CANCEL);
    }
    completion::complete(asid, cap2, OpResult::Cancelled).unwrap();
    completion::close(asid, cap2).unwrap();

    // CQ_WAIT (synthetic, outside thread context): routes and reports pending.
    {
        let mut f = synthetic_trap_frame(asid as u64, 1, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::CQ_WAIT);
        assert_eq!(f.regs[0], 0, "CQ_WAIT should report no pending CQ entries");
    }

    // Mailbox endpoint capabilities.
    let sender_cap = {
        let mut f = synthetic_trap_frame(asid as u64, 0, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_OPEN_SEND);
        assert_ne!(f.regs[0], 0, "MAILBOX_OPEN_SEND should return a capability");
        f.regs[0]
    };
    let recv_cap = {
        let mut f = synthetic_trap_frame(asid as u64, 0, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_OPEN_RECV);
        assert_ne!(f.regs[0], 0, "MAILBOX_OPEN_RECV should return a capability");
        f.regs[0]
    };
    {
        let mut f = synthetic_trap_frame(asid as u64, 0, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_OPEN_RECV);
        assert_eq!(f.regs[0], recv_cap, "MAILBOX_OPEN_RECV should reuse the LP receiver cap");
    }
    {
        let invalid_lp = get_lp_count() as u64;
        let mut f = synthetic_trap_frame(asid as u64, invalid_lp, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_OPEN_SEND);
        assert_eq!(f.regs[0], 0, "MAILBOX_OPEN_SEND should reject invalid target LPs");
    }
    {
        let mut f = synthetic_trap_frame(asid as u64, sender_cap, 0x5a5a, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_SEND_CAP);
        assert_eq!(f.regs[0], 0, "MAILBOX_SEND_CAP should send via a sender capability");
    }
    {
        let mut f = synthetic_trap_frame(asid as u64, recv_cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_RECV_CAP);
        assert_eq!(f.regs[1], 0, "MAILBOX_RECV_CAP should report a message");
        assert_eq!(f.regs[0], 0x5a5a, "MAILBOX_RECV_CAP should return the sent value");
    }
    {
        let mut f = synthetic_trap_frame(asid as u64, recv_cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_SEND_CAP);
        assert_eq!(f.regs[0], 2, "receiver caps must not be usable for send");
    }
    for cap in [sender_cap, recv_cap] {
        let mut f = synthetic_trap_frame(asid as u64, cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_CLOSE);
        assert_eq!(f.regs[0], 0, "MAILBOX_CLOSE should close known caps");
    }
    {
        let mut f = synthetic_trap_frame(asid as u64, sender_cap, 0x6b6b, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_SEND_CAP);
        assert_eq!(f.regs[0], 2, "closed sender caps must be invalid");
    }
    syscall::close_mailbox_address_space(asid);

    completion::close_address_space(asid);
    logln!("Syscall dispatch subsystem tests passed.");
}
