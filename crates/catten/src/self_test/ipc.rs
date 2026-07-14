//! Self-tests for endpoint IPC.

use crate::{
    cpu::isa::{
        interface::memory::{
            address::PhysicalAddress,
            AddressSpaceInterface,
        },
        memory::paging::AddressSpace,
    },
    ipc::{
        self,
        ConnectionRights,
        IpcError,
    },
    logln,
    memory::{
        close_user_address_space,
        object,
        AddressSpaceId,
        VAddr,
        ADDRESS_SPACE_TABLE,
        KERNEL_AS,
    },
};

fn create_ipc_memory_test_address_space(label: &str) -> AddressSpaceId {
    let user_as = {
        let _kas = KERNEL_AS.lock();
        let mut as_ = AddressSpace::get_current();
        #[cfg(target_arch = "aarch64")]
        as_.set_ttbr0(0);
        as_
    };
    let asid = ADDRESS_SPACE_TABLE.lock().add_element(user_as);
    logln!("[ipc memory] {} AS asid={}", label, asid);
    asid
}

pub fn test_endpoint_ipc() {
    logln!("Testing endpoint IPC subsystem...");

    let server = 0x5100;
    let client = 0x5200;

    let endpoint = ipc::endpoint_create(server, 0x4348_4943, 1, 2)
        .expect("endpoint_create should return endpoint cap");
    let connection = ipc::connection_delegate(
        server,
        endpoint,
        client,
        ConnectionRights::SEND | ConnectionRights::CALL,
    )
    .expect("connection_delegate should return client connection cap");

    ipc::scalar_send(client, connection, 7, 0x55).expect("scalar_send should enqueue");
    let message = ipc::receive(server, endpoint).expect("server should receive send message");
    assert_eq!(message.sender, client);
    assert_eq!(message.interface, 0x4348_4943);
    assert_eq!(message.version, 1);
    assert_eq!(message.opcode, 7);
    assert_eq!(message.arg0, 0x55);
    assert_eq!(message.reply, None);

    let call = ipc::scalar_call(client, connection, 8, 0x66)
        .expect("scalar_call should return pending-call cap");
    let message = ipc::receive(server, endpoint).expect("server should receive call message");
    let reply = message.reply.expect("call message should carry reply token cap");
    assert_eq!(message.opcode, 8);
    assert_eq!(message.arg0, 0x66);
    assert_eq!(ipc::poll_reply(client, call), Ok(None));
    ipc::reply(server, reply, -12).expect("reply should complete pending call");
    let value = ipc::poll_reply(client, call).expect("poll_reply should succeed").unwrap();
    assert_eq!(value.result, -12);
    assert_eq!(value.cap, None);
    assert_eq!(
        ipc::reply(server, reply, 13),
        Err(IpcError::UnknownCapability),
        "consumed reply token cap must not be reusable"
    );

    let cancelled_call = ipc::scalar_call(client, connection, 9, 0x77)
        .expect("scalar_call should return cancellable pending-call cap");
    let cancelled_message =
        ipc::receive(server, endpoint).expect("server should receive cancellable call");
    let cancelled_reply =
        cancelled_message.reply.expect("cancellable call message should carry reply token cap");
    ipc::close_cap(client, cancelled_call).expect("client should be able to close pending call");
    assert_eq!(
        ipc::reply(server, cancelled_reply, 14),
        Err(IpcError::UnknownCapability),
        "closing a pending call must invalidate the outstanding reply token"
    );

    let closed_call = ipc::scalar_call(client, connection, 10, 0x88)
        .expect("scalar_call should enqueue call before endpoint close");
    ipc::close_cap(server, endpoint).expect("server should be able to close endpoint");
    let value = ipc::poll_reply(client, closed_call)
        .expect("poll_reply should succeed")
        .expect("closed endpoint should complete call");
    assert_eq!(
        value.result,
        ipc::REPLY_ENDPOINT_CLOSED,
        "closing endpoint must complete queued calls instead of stranding callers"
    );
    assert_eq!(value.cap, None);

    let endpoint = ipc::endpoint_create(server, 0x4348_4943, 1, 2)
        .expect("endpoint_create should return replacement endpoint cap");
    let name_endpoint = ipc::endpoint_create(server, 0x4e41_4d45, 1, 2)
        .expect("endpoint_create should return name-service endpoint cap");
    let name_connection =
        ipc::connection_delegate(server, name_endpoint, client, ConnectionRights::CALL)
            .expect("name-service connection should be delegated to client");
    let connect_call = ipc::scalar_call(client, name_connection, 99, 0)
        .expect("client should be able to call name service");
    let connect_message = ipc::receive(server, name_endpoint)
        .expect("name service should receive connection request");
    let connect_reply = connect_message.reply.expect("call should carry reply token");
    ipc::reply_with_connection(server, connect_reply, endpoint, ConnectionRights::SEND, 0)
        .expect("name service should be able to return a connection cap");
    let value = ipc::poll_reply(client, connect_call)
        .expect("poll_reply should succeed")
        .expect("connection reply should complete call");
    assert_eq!(value.result, 0);
    let returned_connection = value.cap.expect("reply should return delegated connection cap");
    ipc::scalar_send(client, returned_connection, 42, 0xbeef)
        .expect("returned connection should authorize send");
    assert_eq!(
        ipc::scalar_call(client, returned_connection, 43, 0),
        Err(IpcError::PermissionDenied),
        "returned send-only connection must not authorize calls"
    );
    let message = ipc::receive(server, endpoint).expect("server should receive delegated send");
    assert_eq!(message.opcode, 42);
    assert_eq!(message.arg0, 0xbeef);

    let full_endpoint =
        ipc::endpoint_create(server, 0x4655_4c4c, 1, 1).expect("capacity one endpoint");
    let full_connection =
        ipc::connection_delegate(server, full_endpoint, client, ConnectionRights::SEND)
            .expect("send-only connection");
    ipc::scalar_send(client, full_connection, 1, 1).expect("first send should fit");
    assert_eq!(
        ipc::scalar_send(client, full_connection, 2, 2),
        Err(IpcError::QueueFull),
        "second send should fail on full endpoint queue"
    );
    assert_eq!(
        ipc::scalar_call(client, full_connection, 3, 3),
        Err(IpcError::PermissionDenied),
        "send-only connection must not authorize calls"
    );
    assert_eq!(
        ipc::receive(client, connection),
        Err(IpcError::WrongType),
        "client connection cap must not be usable for receive"
    );
    assert_eq!(
        ipc::scalar_send(server, endpoint, 1, 1),
        Err(IpcError::WrongType),
        "endpoint cap must not be usable as a connection"
    );

    let memory_server = create_ipc_memory_test_address_space("server");
    let memory_client = create_ipc_memory_test_address_space("client");
    let memory_endpoint = ipc::endpoint_create(memory_server, 0x4d45_4d49, 1, 4)
        .expect("memory endpoint_create should return endpoint cap");
    let memory_connection = ipc::connection_delegate(
        memory_server,
        memory_endpoint,
        memory_client,
        ConnectionRights::SEND | ConnectionRights::CALL,
    )
    .expect("memory connection_delegate should return client connection cap");

    let cancelled_moved_call =
        object::allocate(memory_client, 1).expect("memory IPC cancel allocation failed");
    let cancelled_pending = ipc::scalar_call_with_memory_move(
        memory_client,
        memory_connection,
        43,
        0x10,
        cancelled_moved_call,
    )
    .expect("memory IPC cancellable call move should enqueue");
    ipc::close_cap(memory_client, cancelled_pending)
        .expect("memory IPC queued pending call close should succeed");
    assert_eq!(
        ipc::receive(memory_server, memory_endpoint),
        Err(IpcError::NoMessage),
        "closing an undelivered memory call should remove the queued request"
    );
    assert_eq!(
        object::info(memory_client, cancelled_moved_call),
        Err(object::MemoryObjectError::UnknownCapability),
        "closing an undelivered moved-memory call consumes the moved object"
    );

    let moved_send = object::allocate(memory_client, 1).expect("memory IPC allocation failed");
    object::map(memory_client, moved_send, VAddr::from(0x40000usize), true)
        .expect("memory IPC client send map failed");
    let client_send_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_client)
        .expect("memory IPC client AS missing")
        .translate_address(VAddr::from(0x40000usize))
        .expect("memory IPC client translation failed");
    unsafe {
        client_send_frame.into_hhdm_mut::<u64>().write_volatile(0x4d45_4d49_5345_4e44);
    }
    object::unmap(memory_client, moved_send).expect("memory IPC client send unmap failed");
    ipc::scalar_send_with_memory_move(memory_client, memory_connection, 44, 0x11, moved_send)
        .expect("memory IPC send move should enqueue");
    assert_eq!(
        object::info(memory_client, moved_send),
        Err(object::MemoryObjectError::UnknownCapability)
    );
    let moved_message =
        ipc::receive(memory_server, memory_endpoint).expect("memory IPC server receive failed");
    let server_moved_cap = moved_message.memory.expect("memory IPC send should carry cap");
    assert_eq!(moved_message.reply, None);
    object::map(memory_server, server_moved_cap, VAddr::from(0x50000usize), false)
        .expect("memory IPC server send map failed");
    let server_send_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_server)
        .expect("memory IPC server AS missing")
        .translate_address(VAddr::from(0x50000usize))
        .expect("memory IPC server translation failed");
    unsafe {
        assert_eq!(server_send_frame.into_hhdm_mut::<u64>().read_volatile(), 0x4d45_4d49_5345_4e44);
    }
    object::unmap(memory_server, server_moved_cap).expect("memory IPC server send unmap failed");
    object::close_cap(memory_server, server_moved_cap)
        .expect("memory IPC server send close failed");

    let moved_call = object::allocate(memory_client, 1).expect("memory IPC call allocation failed");
    object::map(memory_client, moved_call, VAddr::from(0x60000usize), true)
        .expect("memory IPC client call map failed");
    let client_call_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_client)
        .expect("memory IPC client AS missing")
        .translate_address(VAddr::from(0x60000usize))
        .expect("memory IPC client call translation failed");
    unsafe {
        client_call_frame.into_hhdm_mut::<u64>().write_volatile(0x4d45_4d49_4341_4c4c);
    }
    object::unmap(memory_client, moved_call).expect("memory IPC client call unmap failed");
    let memory_call =
        ipc::scalar_call_with_memory_move(memory_client, memory_connection, 45, 0x22, moved_call)
            .expect("memory IPC call move should enqueue");
    let call_message = ipc::receive(memory_server, memory_endpoint)
        .expect("memory IPC server call receive failed");
    let reply = call_message.reply.expect("memory IPC call should carry reply cap");
    let server_call_cap = call_message.memory.expect("memory IPC call should carry memory cap");
    object::map(memory_server, server_call_cap, VAddr::from(0x70000usize), true)
        .expect("memory IPC server call map failed");
    let server_call_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_server)
        .expect("memory IPC server AS missing")
        .translate_address(VAddr::from(0x70000usize))
        .expect("memory IPC server call translation failed");
    unsafe {
        assert_eq!(server_call_frame.into_hhdm_mut::<u64>().read_volatile(), 0x4d45_4d49_4341_4c4c);
        server_call_frame.into_hhdm_mut::<u64>().write_volatile(0x4d45_4d49_444f_4e45);
    }
    object::unmap(memory_server, server_call_cap).expect("memory IPC server call unmap failed");
    ipc::reply_with_memory_move(memory_server, reply, server_call_cap, 123)
        .expect("memory IPC reply move should complete");
    let reply_value = ipc::poll_reply(memory_client, memory_call)
        .expect("memory IPC poll reply should succeed")
        .expect("memory IPC reply should be ready");
    assert_eq!(reply_value.result, 123);
    assert_eq!(reply_value.cap, None);
    let returned_memory = reply_value.memory.expect("memory IPC reply should return memory");
    object::map(memory_client, returned_memory, VAddr::from(0x80000usize), false)
        .expect("memory IPC client returned map failed");
    let returned_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_client)
        .expect("memory IPC client AS missing")
        .translate_address(VAddr::from(0x80000usize))
        .expect("memory IPC returned translation failed");
    unsafe {
        assert_eq!(returned_frame.into_hhdm_mut::<u64>().read_volatile(), 0x4d45_4d49_444f_4e45);
    }
    object::unmap(memory_client, returned_memory).expect("memory IPC client returned unmap failed");
    object::close_cap(memory_client, returned_memory).expect("memory IPC returned close failed");

    let read_borrow =
        object::allocate(memory_client, 1).expect("memory IPC read borrow allocation failed");
    object::map(memory_client, read_borrow, VAddr::from(0x90000usize), true)
        .expect("memory IPC read borrow client map failed");
    let read_borrow_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_client)
        .expect("memory IPC client AS missing")
        .translate_address(VAddr::from(0x90000usize))
        .expect("memory IPC read borrow translation failed");
    unsafe {
        read_borrow_frame.into_hhdm_mut::<u64>().write_volatile(0x5245_4144_424f_5252);
    }
    object::unmap(memory_client, read_borrow).expect("memory IPC read borrow client unmap failed");
    let read_borrow_call = ipc::scalar_call_with_memory_borrow_read(
        memory_client,
        memory_connection,
        46,
        0x33,
        read_borrow,
    )
    .expect("memory IPC read borrow should enqueue");
    let read_borrow_message = ipc::receive(memory_server, memory_endpoint)
        .expect("memory IPC read borrow receive failed");
    let read_borrow_reply = read_borrow_message.reply.expect("read borrow should carry reply");
    let server_read_borrow = read_borrow_message.memory.expect("read borrow should carry memory");
    assert_eq!(
        object::map(memory_server, server_read_borrow, VAddr::from(0xa0000usize), true),
        Err(object::MemoryObjectError::MissingRight),
        "read-borrowed memory must not map writable"
    );
    object::map(memory_server, server_read_borrow, VAddr::from(0xa0000usize), false)
        .expect("memory IPC read borrow server map failed");
    let server_read_borrow_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_server)
        .expect("memory IPC server AS missing")
        .translate_address(VAddr::from(0xa0000usize))
        .expect("memory IPC server read borrow translation failed");
    unsafe {
        assert_eq!(
            server_read_borrow_frame.into_hhdm_mut::<u64>().read_volatile(),
            0x5245_4144_424f_5252
        );
    }
    ipc::reply(memory_server, read_borrow_reply, 124)
        .expect("memory IPC read borrow reply should revoke");
    assert_eq!(
        object::info(memory_server, server_read_borrow),
        Err(object::MemoryObjectError::UnknownCapability),
        "reply should revoke server read-borrow cap"
    );
    assert_eq!(
        ipc::poll_reply(memory_client, read_borrow_call)
            .expect("read borrow poll should succeed")
            .expect("read borrow reply should be ready")
            .result,
        124
    );
    object::map(memory_client, read_borrow, VAddr::from(0xb0000usize), true)
        .expect("memory IPC read borrow owner remap failed");
    object::unmap(memory_client, read_borrow).expect("memory IPC read borrow owner unmap failed");
    object::close_cap(memory_client, read_borrow).expect("memory IPC read borrow close failed");

    let write_borrow =
        object::allocate(memory_client, 1).expect("memory IPC write borrow allocation failed");
    object::map(memory_client, write_borrow, VAddr::from(0xc0000usize), true)
        .expect("memory IPC write borrow client map failed");
    let write_borrow_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_client)
        .expect("memory IPC client AS missing")
        .translate_address(VAddr::from(0xc0000usize))
        .expect("memory IPC write borrow translation failed");
    unsafe {
        write_borrow_frame.into_hhdm_mut::<u64>().write_volatile(0x5752_4954_424f_5252);
    }
    object::unmap(memory_client, write_borrow)
        .expect("memory IPC write borrow client unmap failed");
    let write_borrow_call = ipc::scalar_call_with_memory_borrow_write(
        memory_client,
        memory_connection,
        47,
        0x44,
        write_borrow,
    )
    .expect("memory IPC write borrow should enqueue");
    let write_borrow_message = ipc::receive(memory_server, memory_endpoint)
        .expect("memory IPC write borrow receive failed");
    let write_borrow_reply = write_borrow_message.reply.expect("write borrow should carry reply");
    let server_write_borrow =
        write_borrow_message.memory.expect("write borrow should carry memory");
    object::map(memory_server, server_write_borrow, VAddr::from(0xd0000usize), true)
        .expect("memory IPC write borrow server map failed");
    let server_write_borrow_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_server)
        .expect("memory IPC server AS missing")
        .translate_address(VAddr::from(0xd0000usize))
        .expect("memory IPC server write borrow translation failed");
    unsafe {
        assert_eq!(
            server_write_borrow_frame.into_hhdm_mut::<u64>().read_volatile(),
            0x5752_4954_424f_5252
        );
        server_write_borrow_frame.into_hhdm_mut::<u64>().write_volatile(0x5752_4954_444f_4e45);
    }
    ipc::reply(memory_server, write_borrow_reply, 125)
        .expect("memory IPC write borrow reply should revoke");
    assert_eq!(
        object::info(memory_server, server_write_borrow),
        Err(object::MemoryObjectError::UnknownCapability),
        "reply should revoke server write-borrow cap"
    );
    assert_eq!(
        ipc::poll_reply(memory_client, write_borrow_call)
            .expect("write borrow poll should succeed")
            .expect("write borrow reply should be ready")
            .result,
        125
    );
    object::map(memory_client, write_borrow, VAddr::from(0xe0000usize), false)
        .expect("memory IPC write borrow owner remap failed");
    let write_borrow_returned = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_client)
        .expect("memory IPC client AS missing")
        .translate_address(VAddr::from(0xe0000usize))
        .expect("memory IPC returned write borrow translation failed");
    unsafe {
        assert_eq!(
            write_borrow_returned.into_hhdm_mut::<u64>().read_volatile(),
            0x5752_4954_444f_4e45
        );
    }
    object::unmap(memory_client, write_borrow).expect("memory IPC write borrow owner unmap failed");
    object::close_cap(memory_client, write_borrow).expect("memory IPC write borrow close failed");

    close_user_address_space(memory_client).expect("memory IPC client AS close failed");
    close_user_address_space(memory_server).expect("memory IPC server AS close failed");

    ipc::close_address_space(client);
    ipc::close_address_space(server);
    logln!("Endpoint IPC subsystem tests passed.");
}
