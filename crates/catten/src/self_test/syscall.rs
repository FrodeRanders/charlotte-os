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
        isa::{
            interface::memory::{
                address::PhysicalAddress,
                AddressSpaceInterface,
            },
            lp::LpId,
            memory::paging::AddressSpace,
        },
        multiprocessor::get_lp_count,
    },
    logln,
    memory::{
        close_user_address_space,
        VAddr,
        ADDRESS_SPACE_TABLE,
        KERNEL_AS,
    },
    syscall::{
        self,
        call_no,
        TrapFrame,
    },
};

fn synthetic_trap_frame(x0: u64, x1: u64, x2: u64, x3: u64) -> TrapFrame {
    synthetic_trap_frame_in(crate::memory::KERNEL_ASID, x0, x1, x2, x3)
}

fn synthetic_trap_frame_in(
    asid: crate::memory::AddressSpaceId,
    x0: u64,
    x1: u64,
    x2: u64,
    x3: u64,
) -> TrapFrame {
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
        asid,
    }
}

fn synthetic_trap_frame4_in(
    asid: crate::memory::AddressSpaceId,
    x0: u64,
    x1: u64,
    x2: u64,
    x3: u64,
    x4: u64,
) -> TrapFrame {
    let mut frame = synthetic_trap_frame_in(asid, x0, x1, x2, x3);
    frame.regs[4] = x4;
    frame
}

fn create_syscall_test_address_space(label: &str) -> crate::memory::AddressSpaceId {
    let user_as = {
        let _kas = KERNEL_AS.lock();
        let mut as_ = AddressSpace::get_current();
        #[cfg(target_arch = "aarch64")]
        as_.set_ttbr0(0);
        as_
    };
    let asid = ADDRESS_SPACE_TABLE.lock().add_element(user_as);
    logln!("[syscall memory] {} AS asid={}", label, asid);
    asid
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
    let cap = {
        let mut f = synthetic_trap_frame_in(asid, 0, OpCode::Nop as u64, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_SUBMIT);
        f.regs[0] as usize
    };
    // COMPLETION_COMPLETE
    {
        let mut f = synthetic_trap_frame_in(asid, 0, cap as u64, 42, 0);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_COMPLETE);
    }
    // COMPLETION_POLL
    {
        let mut f = synthetic_trap_frame_in(asid, 0, cap as u64, 0, 0);
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
        let mut f = synthetic_trap_frame_in(asid, 0, cap as u64, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_CLOSE);
    }
    // CANCEL (on a fresh cap)
    let cap2 = completion::submit(asid, OpCode::Write, None).unwrap();
    {
        let mut f = synthetic_trap_frame_in(asid, 0, cap2 as u64, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_CANCEL);
    }
    completion::complete(asid, cap2, OpResult::Cancelled).unwrap();
    completion::close(asid, cap2).unwrap();

    // CQ_WAIT (synthetic, outside thread context): routes and reports pending.
    {
        let mut f = synthetic_trap_frame_in(asid, 0, 1, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::CQ_WAIT);
        assert_eq!(f.regs[0], 0, "CQ_WAIT should report no pending CQ entries");
    }

    // Mailbox endpoint capabilities.
    let sender_cap = {
        let mut f = synthetic_trap_frame_in(asid, 0, 0, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_OPEN_SEND);
        assert_ne!(f.regs[0], 0, "MAILBOX_OPEN_SEND should return a capability");
        f.regs[0]
    };
    let recv_cap = {
        let mut f = synthetic_trap_frame_in(asid, 0, 0, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_OPEN_RECV);
        assert_ne!(f.regs[0], 0, "MAILBOX_OPEN_RECV should return a capability");
        f.regs[0]
    };
    {
        let mut f = synthetic_trap_frame_in(asid, 0, 0, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_OPEN_RECV);
        assert_eq!(f.regs[0], recv_cap, "MAILBOX_OPEN_RECV should reuse the LP receiver cap");
    }
    {
        let invalid_lp = get_lp_count() as u64;
        let mut f = synthetic_trap_frame_in(asid, 0, invalid_lp, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_OPEN_SEND);
        assert_eq!(f.regs[0], 0, "MAILBOX_OPEN_SEND should reject invalid target LPs");
    }
    {
        let mut f = synthetic_trap_frame_in(asid, 0, sender_cap, 0x5a5a, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_SEND_CAP);
        assert_eq!(f.regs[0], 0, "MAILBOX_SEND_CAP should send via a sender capability");
    }
    {
        let mut f = synthetic_trap_frame_in(asid, 0, recv_cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_RECV_CAP);
        assert_eq!(f.regs[1], 0, "MAILBOX_RECV_CAP should report a message");
        assert_eq!(f.regs[0], 0x5a5a, "MAILBOX_RECV_CAP should return the sent value");
    }
    {
        let mut f = synthetic_trap_frame_in(asid, 0, recv_cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_SEND_CAP);
        assert_eq!(f.regs[0], 2, "receiver caps must not be usable for send");
    }
    for cap in [sender_cap, recv_cap] {
        let mut f = synthetic_trap_frame_in(asid, 0, cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_CLOSE);
        assert_eq!(f.regs[0], 0, "MAILBOX_CLOSE should close known caps");
    }
    {
        let mut f = synthetic_trap_frame_in(asid, 0, sender_cap, 0x6b6b, 0);
        syscall::syscall_dispatch(&mut f, call_no::MAILBOX_SEND_CAP);
        assert_eq!(f.regs[0], 2, "closed sender caps must be invalid");
    }
    syscall::close_mailbox_address_space(asid);

    // Endpoint IPC scalar call path.
    let endpoint = {
        let mut f = synthetic_trap_frame_in(asid, 0, 0x5445_5354, 1, 4);
        syscall::syscall_dispatch(&mut f, call_no::IPC_ENDPOINT_CREATE);
        assert_ne!(f.regs[0], 0, "IPC_ENDPOINT_CREATE should return endpoint cap");
        f.regs[0]
    };
    let connection = {
        let rights = crate::ipc::ConnectionRights::SEND | crate::ipc::ConnectionRights::CALL;
        let mut f = synthetic_trap_frame_in(asid, 0, endpoint, rights.bits() as u64, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_CONNECT);
        assert_ne!(f.regs[0], 0, "IPC_CONNECT should return connection cap");
        f.regs[0]
    };
    {
        let mut f = synthetic_trap_frame_in(asid, 0, connection, 11, 0xaa55);
        syscall::syscall_dispatch(&mut f, call_no::IPC_SCALAR_SEND);
        assert_eq!(f.regs[0], 0, "IPC_SCALAR_SEND should succeed");
    }
    {
        let mut f = synthetic_trap_frame_in(asid, 0, endpoint, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_RECV);
        assert_eq!(f.regs[0], 0, "IPC_RECV should return sent message");
        assert_eq!(f.regs[1], 11);
        assert_eq!(f.regs[2], 0xaa55);
        assert_eq!(f.regs[3], 0, "send message should not include reply token");
        assert_eq!(f.regs[4], asid as u64);
        assert_eq!(f.regs[5], 0x5445_5354);
        assert_eq!(f.regs[6], 1);
    }
    let call = {
        let mut f = synthetic_trap_frame_in(asid, 0, connection, 12, 0xbb66);
        syscall::syscall_dispatch(&mut f, call_no::IPC_SCALAR_CALL);
        assert_ne!(f.regs[0], 0, "IPC_SCALAR_CALL should return pending-call cap");
        f.regs[0]
    };
    let reply = {
        let mut f = synthetic_trap_frame_in(asid, 0, endpoint, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_RECV);
        assert_eq!(f.regs[0], 0, "IPC_RECV should return call message");
        assert_eq!(f.regs[1], 12);
        assert_eq!(f.regs[2], 0xbb66);
        assert_ne!(f.regs[3], 0, "call message should include reply token");
        f.regs[3]
    };
    {
        let mut f = synthetic_trap_frame_in(asid, 0, call, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_REPLY_POLL);
        assert_eq!(f.regs[0], 1, "IPC_REPLY_POLL should report pending call");
        assert_eq!(f.regs[2], 0, "pending call should not report a returned cap");
    }
    {
        let mut f = synthetic_trap_frame_in(asid, 0, reply, 77, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_REPLY);
        assert_eq!(f.regs[0], 0, "IPC_REPLY should succeed");
    }
    {
        let mut f = synthetic_trap_frame_in(asid, 0, call, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_REPLY_POLL);
        assert_eq!(f.regs[0], 0, "IPC_REPLY_POLL should report ready call");
        assert_eq!(f.regs[1] as i64, 77);
        assert_eq!(f.regs[2], 0, "plain reply should not report a returned cap");
    }
    let delegated_call = {
        let mut f = synthetic_trap_frame_in(asid, 0, connection, 13, 0xcc77);
        syscall::syscall_dispatch(&mut f, call_no::IPC_SCALAR_CALL);
        assert_ne!(f.regs[0], 0, "IPC_SCALAR_CALL should return delegated pending-call cap");
        f.regs[0]
    };
    let delegated_reply = {
        let mut f = synthetic_trap_frame_in(asid, 0, endpoint, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_RECV);
        assert_eq!(f.regs[0], 0, "IPC_RECV should return delegated call message");
        assert_eq!(f.regs[1], 13);
        assert_ne!(f.regs[3], 0, "delegated call should include reply token");
        f.regs[3]
    };
    {
        let rights = crate::ipc::ConnectionRights::SEND;
        let mut f =
            synthetic_trap_frame_in(asid, 0, delegated_reply, endpoint, rights.bits() as u64);
        syscall::syscall_dispatch(&mut f, call_no::IPC_REPLY_CONNECTION);
        assert_eq!(f.regs[0], 0, "IPC_REPLY_CONNECTION should succeed");
    }
    let delegated_connection = {
        let mut f = synthetic_trap_frame_in(asid, 0, delegated_call, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_REPLY_POLL);
        assert_eq!(f.regs[0], 0, "IPC_REPLY_POLL should report delegated call ready");
        assert_eq!(f.regs[1] as i64, 0);
        assert_ne!(f.regs[2], 0, "delegated reply should return a connection cap");
        f.regs[2]
    };
    {
        let mut f = synthetic_trap_frame_in(asid, 0, delegated_connection, 14, 0xdd88);
        syscall::syscall_dispatch(&mut f, call_no::IPC_SCALAR_SEND);
        assert_eq!(f.regs[0], 0, "delegated connection should authorize send");
    }
    {
        let mut f = synthetic_trap_frame_in(asid, 0, endpoint, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_RECV_BLOCK);
        assert_eq!(f.regs[0], 0, "IPC_RECV_BLOCK should receive queued message");
        assert_eq!(f.regs[1], 14);
        assert_eq!(f.regs[2], 0xdd88);
    }
    {
        let mut f = synthetic_trap_frame_in(asid, 0, delegated_connection, 15, 0xee99);
        syscall::syscall_dispatch(&mut f, call_no::IPC_SCALAR_CALL);
        assert_eq!(f.regs[0], 0, "send-only delegated connection must not authorize calls");
    }
    for cap in [delegated_call, delegated_connection, call, connection, endpoint] {
        let mut f = synthetic_trap_frame_in(asid, 0, cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_CLOSE);
        assert_eq!(f.regs[0], 0, "IPC_CLOSE should close known caps");
    }
    crate::ipc::close_address_space(asid);

    let memory_owner = create_syscall_test_address_space("owner");
    let memory_server = create_syscall_test_address_space("server");
    let memory_cap = {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, 1, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_ALLOC);
        assert_ne!(f.regs[0], 0, "MEMORY_ALLOC should return memory cap");
        f.regs[0]
    };
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, memory_cap, 0x40000, 1);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_MAP);
        assert_eq!(f.regs[0], 0, "MEMORY_MAP should map writable memory");
    }
    let mapped_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_owner)
        .expect("syscall memory owner AS missing")
        .translate_address(VAddr::from(0x40000usize))
        .expect("syscall memory owner translation failed");
    unsafe {
        mapped_frame.into_hhdm_mut::<u64>().write_volatile(0x5359_5343_414c_4c4d);
    }
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, memory_cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_UNMAP);
        assert_eq!(f.regs[0], 0, "MEMORY_UNMAP should unmap memory");
    }
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, memory_cap, 0x41000, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_MAP);
        assert_eq!(f.regs[0], 0, "MEMORY_MAP should remap memory read-only");
    }
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, memory_cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_UNMAP);
        assert_eq!(f.regs[0], 0, "MEMORY_UNMAP should unmap read-only memory");
    }

    let memory_endpoint = {
        let mut f = synthetic_trap_frame_in(memory_server, 0, 0x4d45_4d53, 1, 2);
        syscall::syscall_dispatch(&mut f, call_no::IPC_ENDPOINT_CREATE);
        assert_ne!(f.regs[0], 0, "memory IPC endpoint create should succeed");
        f.regs[0]
    };
    let memory_connection = crate::ipc::connection_delegate(
        memory_server,
        memory_endpoint,
        memory_owner,
        crate::ipc::ConnectionRights::CALL,
    )
    .expect("memory IPC connection delegate should succeed");
    let moved_call = {
        let mut f =
            synthetic_trap_frame4_in(memory_owner, 0, memory_connection, 51, 0x1234, memory_cap);
        syscall::syscall_dispatch(&mut f, call_no::IPC_SCALAR_CALL_MOVE);
        assert_ne!(f.regs[0], 0, "IPC_SCALAR_CALL_MOVE should return pending-call cap");
        f.regs[0]
    };
    let (moved_reply, server_memory_cap) = {
        let mut f = synthetic_trap_frame_in(memory_server, 0, memory_endpoint, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_RECV);
        assert_eq!(f.regs[0], 0, "IPC_RECV should receive moved memory call");
        assert_eq!(f.regs[1], 51);
        assert_ne!(f.regs[3], 0, "moved memory call should include reply token");
        assert_ne!(f.regs[7], 0, "moved memory call should include memory cap");
        (f.regs[3], f.regs[7])
    };
    {
        let mut f = synthetic_trap_frame_in(memory_server, 0, server_memory_cap, 0x50000, 1);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_MAP);
        assert_eq!(f.regs[0], 0, "server MEMORY_MAP should map moved memory");
    }
    let server_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_server)
        .expect("syscall memory server AS missing")
        .translate_address(VAddr::from(0x50000usize))
        .expect("syscall memory server translation failed");
    unsafe {
        assert_eq!(server_frame.into_hhdm_mut::<u64>().read_volatile(), 0x5359_5343_414c_4c4d);
        server_frame.into_hhdm_mut::<u64>().write_volatile(0x5359_5343_444f_4e45);
    }
    {
        let mut f = synthetic_trap_frame_in(memory_server, 0, server_memory_cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_UNMAP);
        assert_eq!(f.regs[0], 0, "server MEMORY_UNMAP should unmap moved memory");
    }
    {
        let mut f = synthetic_trap_frame_in(memory_server, 0, moved_reply, server_memory_cap, 88);
        syscall::syscall_dispatch(&mut f, call_no::IPC_REPLY_MOVE);
        assert_eq!(f.regs[0], 0, "IPC_REPLY_MOVE should return memory to caller");
    }
    let returned_memory = {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, moved_call, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_REPLY_POLL);
        assert_eq!(f.regs[0], 0, "IPC_REPLY_POLL should report moved reply ready");
        assert_eq!(f.regs[1] as i64, 88);
        assert_eq!(f.regs[2], 0, "moved reply should not return a connection cap");
        assert_ne!(f.regs[3], 0, "moved reply should return memory cap");
        f.regs[3]
    };
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, returned_memory, 0x60000, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_MAP);
        assert_eq!(f.regs[0], 0, "owner MEMORY_MAP should map returned memory");
    }
    let returned_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_owner)
        .expect("syscall memory owner AS missing")
        .translate_address(VAddr::from(0x60000usize))
        .expect("syscall memory returned translation failed");
    unsafe {
        assert_eq!(returned_frame.into_hhdm_mut::<u64>().read_volatile(), 0x5359_5343_444f_4e45);
    }
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, returned_memory, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_UNMAP);
        assert_eq!(f.regs[0], 0, "owner MEMORY_UNMAP should unmap returned memory");
    }
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, returned_memory, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_CLOSE);
        assert_eq!(f.regs[0], 0, "MEMORY_CLOSE should close returned memory");
    }
    close_user_address_space(memory_owner).expect("syscall memory owner AS close failed");
    close_user_address_space(memory_server).expect("syscall memory server AS close failed");

    completion::close_address_space(asid);
    logln!("Syscall dispatch subsystem tests passed.");
}
