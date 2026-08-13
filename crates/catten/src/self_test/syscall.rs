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
                AddressSpaceInterface,
                address::PhysicalAddress,
            },
            lp::LpId,
            memory::paging::AddressSpace,
        },
        multiprocessor::get_lp_count,
    },
    logln,
    memory::{
        ADDRESS_SPACE_TABLE,
        KERNEL_AS,
        VAddr,
        object,
    },
    self_test::close_test_address_space,
    syscall::{
        self,
        TrapFrame,
        call_no,
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
        AddressSpace::new_user()
    };
    let asid = ADDRESS_SPACE_TABLE.lock().add_element(user_as);
    logln!("[syscall memory] {} AS asid={}", label, asid);
    asid
}

pub fn test_syscall_dispatch() {
    logln!("Testing syscall dispatch subsystem...");
    let asid = 0xcafe;
    completion::open_address_space(asid, 256);

    // Caller-controlled completion inputs are rejected without entering the
    // scheduler or panicking the kernel.
    {
        let mut invalid_submit = synthetic_trap_frame_in(asid, 0, 99, 0, 0);
        syscall::syscall_dispatch(&mut invalid_submit, call_no::COMPLETION_SUBMIT);
        assert_eq!(
            invalid_submit.regs[0],
            catten_syscall::COMPLETION_SUBMIT_FAILED,
            "unknown completion opcodes must fail non-fatally"
        );
        let mut missing_buffer = synthetic_trap_frame_in(asid, 0, OpCode::Read as u64, 0, 0);
        syscall::syscall_dispatch(&mut missing_buffer, call_no::COMPLETION_SUBMIT);
        assert_eq!(
            missing_buffer.regs[0],
            catten_syscall::COMPLETION_SUBMIT_FAILED,
            "completion Read must reject a missing destination buffer"
        );
        let mut invalid_poll = synthetic_trap_frame_in(asid, 0, u64::MAX, 0, 0);
        syscall::syscall_dispatch(&mut invalid_poll, call_no::COMPLETION_POLL);
        assert_eq!(
            invalid_poll.regs[0],
            catten_syscall::completion_status::INVALID_CAPABILITY,
            "poll must distinguish an invalid capability from pending"
        );
        let mut invalid_wait = synthetic_trap_frame_in(asid, 0, u64::MAX, 1, 0);
        syscall::syscall_dispatch(&mut invalid_wait, call_no::COMPLETION_WAIT_TIMEOUT);
        assert_eq!(
            invalid_wait.regs[0],
            catten_syscall::completion_status::INVALID_CAPABILITY,
            "wait_timeout must distinguish an invalid capability from timeout"
        );
    }

    // LOG
    {
        let mut f = synthetic_trap_frame(0xdead, 0xbeef, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::LOG);
    }
    // COMPLETION_SUBMIT
    let cap = {
        let mut f = synthetic_trap_frame_in(asid, 0, OpCode::Nop as u64, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_SUBMIT);
        f.regs[0]
    };
    assert!(crate::capability::contains(asid, cap, crate::capability::ObjectKind::Completion));
    assert!(!crate::capability::contains(asid, cap, crate::capability::ObjectKind::Ipc));
    assert_eq!(
        crate::ipc::receive(asid, cap),
        Err(crate::ipc::IpcError::UnknownCapability),
        "a completion capability must not be accepted as IPC authority"
    );
    // COMPLETION_COMPLETE
    {
        let mut f = synthetic_trap_frame_in(asid, 0, cap, 42, 0);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_COMPLETE);
    }
    // COMPLETION_POLL
    {
        let mut f = synthetic_trap_frame_in(asid, 0, cap, 0, 0);
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
        let mut f = synthetic_trap_frame_in(asid, 0, cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_CLOSE);
    }
    // CANCEL (on a fresh cap)
    let cap2 = completion::submit(asid, OpCode::Write, None).unwrap();
    {
        let mut f = synthetic_trap_frame_in(asid, 0, cap2, 0, 0);
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
    for cap in [delegated_connection, delegated_call, call, connection, endpoint] {
        let mut f = synthetic_trap_frame_in(asid, 0, cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_CLOSE);
        assert_eq!(f.regs[0], 0, "IPC_CLOSE should close known caps");
    }
    crate::ipc::close_address_space(asid);

    let memory_owner = create_syscall_test_address_space("owner");
    let memory_server = create_syscall_test_address_space("server");
    completion::open_address_space(memory_owner, 32);

    // A completion Read validates EL0 write permission for every destination
    // byte and supports an unaligned write that crosses into a separately
    // translated page.
    let user_copy_cap =
        object::allocate(memory_owner, 2).expect("checked user-copy object allocation failed");
    let user_copy_base = VAddr::from(0x20_0000usize);
    object::map(memory_owner, user_copy_cap, user_copy_base, true)
        .expect("checked user-copy mapping failed");
    let cross_page = 0x20_0fffusize;
    let user_copy_completion = {
        let mut f =
            synthetic_trap_frame_in(memory_owner, 0, OpCode::Read as u64, cross_page as u64, 4);
        syscall::syscall_dispatch(&mut f, call_no::COMPLETION_SUBMIT);
        assert_ne!(f.regs[0], catten_syscall::COMPLETION_SUBMIT_FAILED);
        f.regs[0]
    };
    let completed = completion::poll(memory_owner, user_copy_completion)
        .expect("checked user-copy poll failed")
        .expect("checked user-copy completion should be terminal");
    assert_eq!(completed.result, OpResult::Ok(4));
    completion::close(memory_owner, user_copy_completion)
        .expect("checked user-copy completion close failed");
    let expected = 0xfeed_f00du32.to_ne_bytes();
    for (offset, expected_byte) in expected.into_iter().enumerate() {
        let frame = ADDRESS_SPACE_TABLE
            .lock()
            .get_mut(memory_owner)
            .expect("checked user-copy AS missing")
            .translate_address(VAddr::from(cross_page + offset))
            .expect("checked user-copy byte should be mapped");
        unsafe {
            assert_eq!(frame.into_hhdm_mut::<u8>().read_volatile(), expected_byte);
        }
    }
    object::unmap(memory_owner, user_copy_cap).expect("checked user-copy writable unmap failed");
    let read_only_base = VAddr::from(0x21_0000usize);
    object::map(memory_owner, user_copy_cap, read_only_base, false)
        .expect("checked user-copy read-only map failed");
    let mut read_only = synthetic_trap_frame_in(
        memory_owner,
        0,
        OpCode::Read as u64,
        usize::from(read_only_base) as u64,
        4,
    );
    syscall::syscall_dispatch(&mut read_only, call_no::COMPLETION_SUBMIT);
    assert_eq!(
        read_only.regs[0],
        catten_syscall::COMPLETION_SUBMIT_FAILED,
        "completion Read must not bypass a read-only user mapping"
    );
    object::unmap(memory_owner, user_copy_cap).expect("checked user-copy read-only unmap failed");
    object::close_cap(memory_owner, user_copy_cap).expect("checked user-copy object close failed");

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

    let copy_cap = {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, 1, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_ALLOC);
        assert_ne!(f.regs[0], 0, "copy MEMORY_ALLOC should return cap");
        f.regs[0]
    };
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, copy_cap, 0x70000, 1);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_MAP);
        assert_eq!(f.regs[0], 0, "copy MEMORY_MAP should map original");
    }
    let copy_seed_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_owner)
        .expect("syscall memory owner AS missing")
        .translate_address(VAddr::from(0x70000usize))
        .expect("syscall memory copy seed translation failed");
    unsafe {
        copy_seed_frame.into_hhdm_mut::<u64>().write_volatile(0x5359_5343_434f_5059);
    }
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, copy_cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_UNMAP);
        assert_eq!(f.regs[0], 0, "copy MEMORY_UNMAP should unmap original");
    }
    let copy_call = {
        let mut f =
            synthetic_trap_frame4_in(memory_owner, 0, memory_connection, 52, 0x5678, copy_cap);
        syscall::syscall_dispatch(&mut f, call_no::IPC_SCALAR_CALL_COPY);
        assert_ne!(f.regs[0], 0, "IPC_SCALAR_CALL_COPY should return pending-call cap");
        f.regs[0]
    };
    let (copy_reply, server_copy_cap) = {
        let mut f = synthetic_trap_frame_in(memory_server, 0, memory_endpoint, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_RECV);
        assert_eq!(f.regs[0], 0, "IPC_RECV should receive copied memory call");
        assert_eq!(f.regs[1], 52);
        assert_ne!(f.regs[3], 0, "copied memory call should include reply token");
        assert_ne!(f.regs[7], 0, "copied memory call should include memory cap");
        (f.regs[3], f.regs[7])
    };
    {
        let mut f = synthetic_trap_frame_in(memory_server, 0, server_copy_cap, 0x80000, 1);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_MAP);
        assert_eq!(f.regs[0], 0, "server MEMORY_MAP should map copied memory");
    }
    let server_copy_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_server)
        .expect("syscall memory server AS missing")
        .translate_address(VAddr::from(0x80000usize))
        .expect("syscall memory copy server translation failed");
    unsafe {
        assert_eq!(server_copy_frame.into_hhdm_mut::<u64>().read_volatile(), 0x5359_5343_434f_5059);
        server_copy_frame.into_hhdm_mut::<u64>().write_volatile(0x5359_5343_434f_5032);
    }
    {
        let mut f = synthetic_trap_frame_in(memory_server, 0, server_copy_cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_UNMAP);
        assert_eq!(f.regs[0], 0, "server MEMORY_UNMAP should unmap copied memory");
    }
    {
        let mut f = synthetic_trap_frame_in(memory_server, 0, server_copy_cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_CLOSE);
        assert_eq!(f.regs[0], 0, "server MEMORY_CLOSE should close copied memory");
    }
    {
        let mut f = synthetic_trap_frame_in(memory_server, 0, copy_reply, 89, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_REPLY);
        assert_eq!(f.regs[0], 0, "IPC_REPLY should complete copied memory call");
    }
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, copy_call, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::IPC_REPLY_POLL);
        assert_eq!(f.regs[0], 0, "IPC_REPLY_POLL should report copy reply ready");
        assert_eq!(f.regs[1] as i64, 89);
        assert_eq!(f.regs[2], 0, "copy reply should not return a connection cap");
        assert_eq!(f.regs[3], 0, "copy reply should not return a memory cap");
    }
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, copy_cap, 0x90000, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_MAP);
        assert_eq!(f.regs[0], 0, "owner MEMORY_MAP should still map original after copy");
    }
    let copy_original_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_owner)
        .expect("syscall memory owner AS missing")
        .translate_address(VAddr::from(0x90000usize))
        .expect("syscall memory copy original translation failed");
    unsafe {
        assert_eq!(
            copy_original_frame.into_hhdm_mut::<u64>().read_volatile(),
            0x5359_5343_434f_5059
        );
    }
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, copy_cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_UNMAP);
        assert_eq!(f.regs[0], 0, "owner MEMORY_UNMAP should unmap original copy source");
    }
    {
        let mut f = synthetic_trap_frame_in(memory_owner, 0, copy_cap, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::MEMORY_CLOSE);
        assert_eq!(f.regs[0], 0, "owner MEMORY_CLOSE should close original copy source");
    }

    #[cfg(target_arch = "aarch64")]
    {
        use catten_syscall::{
            THREAD_STATISTICS_HEADER_U64S,
            THREAD_STATISTICS_MAGIC,
            THREAD_STATISTICS_VERSION,
        };

        let mut f = synthetic_trap_frame_in(memory_owner, 0, 0, 0, 0);
        syscall::syscall_dispatch(&mut f, call_no::THREAD_STATISTICS);
        let statistics_cap = f.regs[0];
        let length = usize::try_from(f.regs[1]).expect("statistics length exceeds usize");
        assert_ne!(statistics_cap, 0, "THREAD_STATISTICS should return a memory object");
        assert!(
            length >= THREAD_STATISTICS_HEADER_U64S * size_of::<u64>(),
            "THREAD_STATISTICS should return a complete header"
        );
        let bytes = crate::memory::object::snapshot_bytes(memory_owner, statistics_cap, length)
            .expect("statistics snapshot should be readable by its owner");
        let field = |index: usize| {
            u64::from_le_bytes(
                bytes[index * size_of::<u64>()..(index + 1) * size_of::<u64>()]
                    .try_into()
                    .expect("statistics field should contain one u64"),
            )
        };
        assert_eq!(field(0), THREAD_STATISTICS_MAGIC);
        assert_eq!(field(1), THREAD_STATISTICS_VERSION);
        assert_eq!(
            usize::try_from(field(3)).expect("record count exceeds usize")
                * usize::try_from(field(2)).expect("record size exceeds usize")
                + THREAD_STATISTICS_HEADER_U64S * size_of::<u64>(),
            length,
            "THREAD_STATISTICS exact length should match its header"
        );
        assert_ne!(field(4), 0, "statistics counter frequency should be reported");
        crate::memory::object::close_cap(memory_owner, statistics_cap)
            .expect("statistics memory object should close");

        let mut invalid = synthetic_trap_frame_in(memory_owner, 0, u64::MAX, 0, 0);
        syscall::syscall_dispatch(&mut invalid, call_no::THREAD_STATISTICS);
        assert_eq!(
            (invalid.regs[0], invalid.regs[1]),
            (0, 0),
            "an ungranted observer capability must not widen the snapshot"
        );

        let observer_cap = crate::capability::allocate(
            memory_owner,
            crate::capability::ObjectKind::SystemObserver,
        );
        let mut system = synthetic_trap_frame_in(memory_owner, 0, observer_cap, 0, 0);
        syscall::syscall_dispatch(&mut system, call_no::THREAD_STATISTICS);
        assert_ne!(system.regs[0], 0, "delegated observer should receive a snapshot");
        let system_bytes = crate::memory::object::snapshot_bytes(
            memory_owner,
            system.regs[0],
            usize::try_from(system.regs[1]).expect("system statistics length exceeds usize"),
        )
        .expect("system statistics snapshot should be readable");
        let system_count = u64::from_le_bytes(
            system_bytes[3 * size_of::<u64>()..4 * size_of::<u64>()]
                .try_into()
                .expect("system statistics count should contain one u64"),
        );
        assert_ne!(system_count, 0, "system observer should see scheduler threads");
        crate::memory::object::close_cap(memory_owner, system.regs[0])
            .expect("system statistics memory object should close");
        assert!(crate::capability::remove(
            memory_owner,
            observer_cap,
            crate::capability::ObjectKind::SystemObserver
        ));
    }

    close_test_address_space(memory_owner).expect("syscall memory owner AS close failed");
    close_test_address_space(memory_server).expect("syscall memory server AS close failed");

    completion::close_address_space(asid);
    logln!("Syscall dispatch subsystem tests passed.");
}
