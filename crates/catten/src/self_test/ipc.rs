//! Self-tests for endpoint IPC.

use crate::{
    cpu::isa::{
        interface::memory::{
            AddressSpaceInterface,
            address::PhysicalAddress,
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
        ADDRESS_SPACE_TABLE,
        AddressSpaceId,
        KERNEL_AS,
        VAddr,
        close_user_address_space,
        object,
    },
};

fn create_ipc_memory_test_address_space(label: &str) -> AddressSpaceId {
    let user_as = {
        let _kas = KERNEL_AS.lock();
        #[cfg_attr(not(target_arch = "aarch64"), allow(unused_mut))]
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

    let copied_send =
        object::allocate(memory_client, 1).expect("memory IPC copy allocation failed");
    object::map(memory_client, copied_send, VAddr::from(0x51000usize), true)
        .expect("memory IPC client copy map failed");
    let client_copy_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_client)
        .expect("memory IPC client AS missing")
        .translate_address(VAddr::from(0x51000usize))
        .expect("memory IPC client copy translation failed");
    unsafe {
        client_copy_frame.into_hhdm_mut::<u64>().write_volatile(0x4d45_4d49_434f_5059);
    }
    object::unmap(memory_client, copied_send).expect("memory IPC client copy unmap failed");
    ipc::scalar_send_with_memory_copy(memory_client, memory_connection, 45, 0x12, copied_send)
        .expect("memory IPC send copy should enqueue");
    object::map(memory_client, copied_send, VAddr::from(0x52000usize), false)
        .expect("memory IPC original copy remap failed");
    let original_copy_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_client)
        .expect("memory IPC client AS missing")
        .translate_address(VAddr::from(0x52000usize))
        .expect("memory IPC original copy translation failed");
    unsafe {
        assert_eq!(
            original_copy_frame.into_hhdm_mut::<u64>().read_volatile(),
            0x4d45_4d49_434f_5059
        );
    }
    object::unmap(memory_client, copied_send).expect("memory IPC original copy unmap failed");
    let copied_message =
        ipc::receive(memory_server, memory_endpoint).expect("memory IPC copy receive failed");
    let server_copy_cap = copied_message.memory.expect("memory IPC copy should carry cap");
    assert_eq!(copied_message.reply, None);
    object::map(memory_server, server_copy_cap, VAddr::from(0x53000usize), true)
        .expect("memory IPC server copy map failed");
    let server_copy_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_server)
        .expect("memory IPC server AS missing")
        .translate_address(VAddr::from(0x53000usize))
        .expect("memory IPC server copy translation failed");
    unsafe {
        assert_eq!(server_copy_frame.into_hhdm_mut::<u64>().read_volatile(), 0x4d45_4d49_434f_5059);
        server_copy_frame.into_hhdm_mut::<u64>().write_volatile(0x4d45_4d49_434f_5032);
    }
    object::unmap(memory_server, server_copy_cap).expect("memory IPC server copy unmap failed");
    object::close_cap(memory_server, server_copy_cap).expect("memory IPC server copy close failed");
    object::map(memory_client, copied_send, VAddr::from(0x54000usize), false)
        .expect("memory IPC original copy remap after receiver write failed");
    let original_after_copy_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(memory_client)
        .expect("memory IPC client AS missing")
        .translate_address(VAddr::from(0x54000usize))
        .expect("memory IPC original after copy translation failed");
    unsafe {
        assert_eq!(
            original_after_copy_frame.into_hhdm_mut::<u64>().read_volatile(),
            0x4d45_4d49_434f_5059,
            "receiver writes to a copied memory object must not modify the sender original"
        );
    }
    object::unmap(memory_client, copied_send).expect("memory IPC original copy final unmap failed");
    object::close_cap(memory_client, copied_send).expect("memory IPC original copy close failed");

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

    let teardown_server = create_ipc_memory_test_address_space("teardown server");
    let teardown_client = create_ipc_memory_test_address_space("teardown client");
    let teardown_endpoint = ipc::endpoint_create(teardown_server, 0x4d45_4d54, 1, 4)
        .expect("memory IPC teardown endpoint_create failed");
    let teardown_connection = ipc::connection_delegate(
        teardown_server,
        teardown_endpoint,
        teardown_client,
        ConnectionRights::CALL,
    )
    .expect("memory IPC teardown connection_delegate failed");

    let teardown_moved =
        object::allocate(teardown_client, 1).expect("memory IPC teardown move allocation failed");
    let teardown_call = ipc::scalar_call_with_memory_move(
        teardown_client,
        teardown_connection,
        48,
        0x55,
        teardown_moved,
    )
    .expect("memory IPC teardown moved call failed");
    close_user_address_space(teardown_server).expect("memory IPC teardown server AS close failed");
    let teardown_reply = ipc::poll_reply(teardown_client, teardown_call)
        .expect("memory IPC teardown moved poll failed")
        .expect("memory IPC teardown moved call should complete");
    assert_eq!(
        teardown_reply.result,
        ipc::REPLY_ENDPOINT_CLOSED,
        "server teardown must complete queued moved-memory calls as endpoint-closed"
    );
    assert_eq!(teardown_reply.memory, None);
    assert_eq!(
        object::info(teardown_client, teardown_moved),
        Err(object::MemoryObjectError::UnknownCapability),
        "server teardown must not resurrect moved-from memory caps"
    );
    assert_eq!(
        ipc::scalar_call(teardown_client, teardown_connection, 49, 0x56),
        Err(IpcError::EndpointClosed),
        "connection to torn-down endpoint must report endpoint closed"
    );
    close_user_address_space(teardown_client).expect("memory IPC teardown client AS close failed");

    let death_server = create_ipc_memory_test_address_space("death server");
    let death_client = create_ipc_memory_test_address_space("death client");
    let death_endpoint = ipc::endpoint_create(death_server, 0x4d45_4454, 1, 4)
        .expect("memory IPC death endpoint_create failed");
    let death_connection = ipc::connection_delegate(
        death_server,
        death_endpoint,
        death_client,
        ConnectionRights::CALL,
    )
    .expect("memory IPC death connection_delegate failed");

    let death_borrow =
        object::allocate(death_client, 1).expect("memory IPC death borrow allocation failed");
    object::map(death_client, death_borrow, VAddr::from(0xf0000usize), true)
        .expect("memory IPC death client seed map failed");
    let death_seed_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(death_client)
        .expect("memory IPC death client AS missing")
        .translate_address(VAddr::from(0xf0000usize))
        .expect("memory IPC death seed translation failed");
    unsafe {
        death_seed_frame.into_hhdm_mut::<u64>().write_volatile(0x4445_4154_4849_4e49);
    }
    object::unmap(death_client, death_borrow).expect("memory IPC death client seed unmap failed");
    let death_call = ipc::scalar_call_with_memory_borrow_write(
        death_client,
        death_connection,
        50,
        0x57,
        death_borrow,
    )
    .expect("memory IPC death write borrow call failed");
    let death_message =
        ipc::receive(death_server, death_endpoint).expect("memory IPC death receive failed");
    let death_server_borrow =
        death_message.memory.expect("memory IPC death call should carry memory");
    object::map(death_server, death_server_borrow, VAddr::from(0x100000usize), true)
        .expect("memory IPC death server borrow map failed");
    let death_server_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(death_server)
        .expect("memory IPC death server AS missing")
        .translate_address(VAddr::from(0x100000usize))
        .expect("memory IPC death server translation failed");
    unsafe {
        assert_eq!(
            death_server_frame.into_hhdm_mut::<u64>().read_volatile(),
            0x4445_4154_4849_4e49
        );
        death_server_frame.into_hhdm_mut::<u64>().write_volatile(0x4445_4154_4844_4f4e);
    }
    close_user_address_space(death_server).expect("memory IPC death server AS close failed");
    let death_reply = ipc::poll_reply(death_client, death_call)
        .expect("memory IPC death poll failed")
        .expect("memory IPC death delivered call should complete");
    assert_eq!(
        death_reply.result,
        ipc::REPLY_CANCELLED,
        "server death must cancel delivered calls with outstanding reply tokens"
    );
    object::map(death_client, death_borrow, VAddr::from(0x110000usize), true)
        .expect("memory IPC death owner remap after server death failed");
    let death_owner_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(death_client)
        .expect("memory IPC death client AS missing")
        .translate_address(VAddr::from(0x110000usize))
        .expect("memory IPC death owner translation failed");
    unsafe {
        assert_eq!(death_owner_frame.into_hhdm_mut::<u64>().read_volatile(), 0x4445_4154_4844_4f4e);
    }
    object::unmap(death_client, death_borrow)
        .expect("memory IPC death owner unmap after server death failed");
    object::close_cap(death_client, death_borrow)
        .expect("memory IPC death owner close after server death failed");
    close_user_address_space(death_client).expect("memory IPC death client AS close failed");

    ipc::close_address_space(client);
    ipc::close_address_space(server);
    logln!("Endpoint IPC subsystem tests passed.");
}

/// Exercises call-time connection attachment and re-delegation: the
/// name-service authority pattern from Phase 3.
///
/// A "service" domain owns an endpoint and hands a re-delegable connection to
/// a "name service" domain by attaching it to a call. The name service later
/// returns attenuated connections to a "client" domain at reply time, minted
/// from the connection it holds rather than from an endpoint it owns.
pub fn test_endpoint_ipc_connection_attach() {
    logln!("Testing endpoint IPC connection attachment and re-delegation...");

    let nameservice = 0x6100;
    let service = 0x6200;
    let client = 0x6300;

    let ns_endpoint = ipc::endpoint_create(nameservice, 0x4e53_5643, 1, 4)
        .expect("name-service endpoint_create failed");
    let service_ns_conn =
        ipc::connection_delegate(nameservice, ns_endpoint, service, ConnectionRights::CALL)
            .expect("service bootstrap connection delegation failed");
    let client_ns_conn =
        ipc::connection_delegate(nameservice, ns_endpoint, client, ConnectionRights::CALL)
            .expect("client bootstrap connection delegation failed");

    let service_endpoint =
        ipc::endpoint_create(service, 0x4543_484f, 1, 4).expect("service endpoint_create failed");

    // Attaching a cap without MINT_CONNECTION must fail.
    let weak_conn = ipc::connection_mint(service, service_endpoint, ConnectionRights::SEND)
        .expect("weak connection mint failed");
    assert_eq!(
        ipc::scalar_call_with_connection(
            service,
            service_ns_conn,
            1,
            0x1111,
            weak_conn,
            ConnectionRights::SEND,
        ),
        Err(IpcError::PermissionDenied),
        "attaching a non-mintable connection must be denied"
    );
    ipc::close_cap(service, weak_conn).expect("weak connection close failed");

    // Attaching a wrong-type cap must fail.
    let bogus_call =
        ipc::scalar_call(service, service_ns_conn, 2, 0).expect("bogus scalar_call failed");
    assert_eq!(
        ipc::scalar_call_with_connection(
            service,
            service_ns_conn,
            1,
            0x1111,
            bogus_call,
            ConnectionRights::SEND,
        ),
        Err(IpcError::WrongType),
        "attaching a pending-call cap must be rejected"
    );
    let bogus_message =
        ipc::receive(nameservice, ns_endpoint).expect("bogus registration receive failed");
    assert_eq!(bogus_message.opcode, 2);
    assert_eq!(bogus_message.connection, None);
    ipc::close_cap(service, bogus_call).expect("bogus pending call close failed");

    // REGISTER: attach a re-delegable connection to the service endpoint.
    let register_call = ipc::scalar_call_with_connection(
        service,
        service_ns_conn,
        1,
        0x6563_686f, // "echo"
        service_endpoint,
        ConnectionRights::SEND | ConnectionRights::CALL | ConnectionRights::MINT_CONNECTION,
    )
    .expect("register call with connection failed");
    let register_message = ipc::receive(nameservice, ns_endpoint).expect("register receive failed");
    assert_eq!(register_message.opcode, 1);
    assert_eq!(register_message.arg0, 0x6563_686f);
    let stored_conn =
        register_message.connection.expect("register message should carry attached connection cap");
    let register_reply = register_message.reply.expect("register should carry reply token");
    ipc::reply(nameservice, register_reply, 1).expect("register reply failed");
    let register_value = ipc::poll_reply(service, register_call)
        .expect("register poll failed")
        .expect("register reply should be ready");
    assert_eq!(register_value.result, 1, "registration should report generation 1");

    // LOOKUP: reply with a connection minted from the stored connection cap.
    let lookup_call =
        ipc::scalar_call(client, client_ns_conn, 3, 0x6563_686f).expect("lookup call failed");
    let lookup_message = ipc::receive(nameservice, ns_endpoint).expect("lookup receive failed");
    assert_eq!(lookup_message.opcode, 3);
    let lookup_reply = lookup_message.reply.expect("lookup should carry reply token");
    ipc::reply_with_connection(
        nameservice,
        lookup_reply,
        stored_conn,
        ConnectionRights::SEND | ConnectionRights::CALL,
        1,
    )
    .expect("lookup reply with re-delegated connection failed");
    let lookup_value = ipc::poll_reply(client, lookup_call)
        .expect("lookup poll failed")
        .expect("lookup reply should be ready");
    assert_eq!(lookup_value.result, 1);
    let client_service_conn =
        lookup_value.cap.expect("lookup should return delegated service connection");

    // The delegated connection reaches the real service endpoint...
    let echo_call = ipc::scalar_call(client, client_service_conn, 7, 0xbeef)
        .expect("client call through re-delegated connection failed");
    let echo_message = ipc::receive(service, service_endpoint).expect("service receive failed");
    assert_eq!(echo_message.opcode, 7);
    assert_eq!(echo_message.arg0, 0xbeef);
    ipc::reply(service, echo_message.reply.expect("echo should carry reply"), 0xbeef)
        .expect("echo reply failed");
    assert_eq!(
        ipc::poll_reply(client, echo_call)
            .expect("echo poll failed")
            .expect("echo reply should be ready")
            .result,
        0xbeef
    );

    // ...but must not be re-delegable or receivable itself (attenuation).
    assert_eq!(
        ipc::connection_mint(client, client_service_conn, ConnectionRights::SEND),
        Err(IpcError::PermissionDenied),
        "attenuated connection must not mint further connections"
    );
    assert_eq!(
        ipc::receive(client, client_service_conn),
        Err(IpcError::WrongType),
        "delegated connection must not be usable for receive"
    );

    // Cancelling a queued connection-bearing call must remove the queued
    // message so the receiver never observes it.
    let cancelled = ipc::scalar_call_with_connection(
        service,
        service_ns_conn,
        1,
        0x6c6f_67, // "log"
        service_endpoint,
        ConnectionRights::SEND | ConnectionRights::MINT_CONNECTION,
    )
    .expect("cancellable register call failed");
    ipc::close_cap(service, cancelled).expect("cancellable register close failed");
    assert_eq!(
        ipc::receive(nameservice, ns_endpoint),
        Err(IpcError::NoMessage),
        "cancelled connection-bearing call must not be delivered"
    );

    // Closing an endpoint with queued connection-bearing calls must complete
    // them as endpoint-closed.
    let doomed_call = ipc::scalar_call_with_connection(
        service,
        service_ns_conn,
        1,
        0x74_696d, // "tim"
        service_endpoint,
        ConnectionRights::SEND | ConnectionRights::MINT_CONNECTION,
    )
    .expect("doomed register call failed");
    ipc::close_cap(nameservice, ns_endpoint).expect("name-service endpoint close failed");
    let doomed_value = ipc::poll_reply(service, doomed_call)
        .expect("doomed poll failed")
        .expect("endpoint close should complete queued register call");
    assert_eq!(doomed_value.result, ipc::REPLY_ENDPOINT_CLOSED);

    // Stale service connections fail deterministically after the service
    // endpoint closes (restart semantics).
    ipc::close_cap(service, service_endpoint).expect("service endpoint close failed");
    assert_eq!(
        ipc::scalar_call(client, client_service_conn, 8, 0),
        Err(IpcError::EndpointClosed),
        "connection to restarted service must report endpoint closed"
    );

    ipc::close_address_space(client);
    ipc::close_address_space(service);
    ipc::close_address_space(nameservice);
    logln!("Endpoint IPC connection attachment tests passed.");
}

/// Exercises the combined connection + copied-memory attachment primitive
/// (`scalar_call_with_connection_copy`), the kernel mechanism that lets a
/// service register under a memory-carried (long) name in one call.
pub fn test_endpoint_ipc_connection_copy() {
    logln!("Testing combined connection + copied-memory IPC attachment...");

    let nameservice = create_ipc_memory_test_address_space("named-ns");
    let service = create_ipc_memory_test_address_space("named-service");

    let ns_endpoint = ipc::endpoint_create(nameservice, 0x4e53_5632, 1, 4)
        .expect("named name-service endpoint_create failed");
    let service_ns_conn =
        ipc::connection_delegate(nameservice, ns_endpoint, service, ConnectionRights::CALL)
            .expect("named service bootstrap connection delegation failed");
    let service_endpoint = ipc::endpoint_create(service, 0x4543_4832, 1, 4)
        .expect("named service endpoint_create failed");

    // The service writes a long name into a memory object, then registers
    // with a combined connection + copied-name call.
    let name_bytes: &[u8] = b"system.console.primary.v1";
    let name_object = object::allocate(service, 1).expect("named name allocation failed");
    object::map(service, name_object, VAddr::from(0x120000usize), true)
        .expect("named name map failed");
    let name_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(service)
        .expect("named service AS missing")
        .translate_address(VAddr::from(0x120000usize))
        .expect("named name translation failed");
    unsafe {
        let dst: *mut u8 = name_frame.into();
        core::ptr::copy_nonoverlapping(name_bytes.as_ptr(), dst, name_bytes.len());
    }
    object::unmap(service, name_object).expect("named name unmap failed");

    let register = ipc::scalar_call_with_connection_copy(
        service,
        service_ns_conn,
        1,
        name_bytes.len() as u64,
        service_endpoint,
        ConnectionRights::SEND | ConnectionRights::CALL | ConnectionRights::MINT_CONNECTION,
        name_object,
    )
    .expect("named combined register call failed");

    // The copy preserves the sender's ownership.
    assert!(
        object::info(service, name_object).is_ok(),
        "copy attachment must not consume the sender's name object"
    );

    let message = ipc::receive(nameservice, ns_endpoint).expect("named register receive failed");
    assert_eq!(message.arg0, name_bytes.len() as u64, "name length should arrive in arg0");
    let stored_conn = message.connection.expect("combined call should carry attached connection");
    let name_copy = message.memory.expect("combined call should carry copied name memory");
    let reply = message.reply.expect("combined call should carry reply token");

    // The name service reads the copied name and verifies it.
    object::map(nameservice, name_copy, VAddr::from(0x130000usize), false)
        .expect("named copy map failed");
    let copy_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(nameservice)
        .expect("named ns AS missing")
        .translate_address(VAddr::from(0x130000usize))
        .expect("named copy translation failed");
    unsafe {
        let src: *const u8 = copy_frame.into();
        for (i, expected) in name_bytes.iter().enumerate() {
            assert_eq!(
                core::ptr::read_volatile(src.add(i)),
                *expected,
                "copied name byte {} mismatch",
                i
            );
        }
    }
    object::unmap(nameservice, name_copy).expect("named copy unmap failed");
    object::close_cap(nameservice, name_copy).expect("named copy close failed");

    ipc::reply(nameservice, reply, 1).expect("named register reply failed");
    let value = ipc::poll_reply(service, register)
        .expect("named register poll failed")
        .expect("named register reply should be ready");
    assert_eq!(value.result, 1, "named registration should report generation 1");

    // The stored connection reaches the real service endpoint.
    ipc::scalar_send(nameservice, stored_conn, 5, 0xabc)
        .expect("stored named connection should authorize send");
    let echoed = ipc::receive(service, service_endpoint)
        .expect("named service should receive delegated send");
    assert_eq!(echoed.opcode, 5);
    assert_eq!(echoed.arg0, 0xabc);

    // Cancelling a queued combined call reclaims both attachments.
    let cancel_name = object::allocate(service, 1).expect("cancel name allocation failed");
    let cancelled = ipc::scalar_call_with_connection_copy(
        service,
        service_ns_conn,
        1,
        4,
        service_endpoint,
        ConnectionRights::SEND | ConnectionRights::MINT_CONNECTION,
        cancel_name,
    )
    .expect("cancellable combined register call failed");
    ipc::close_cap(service, cancelled).expect("cancellable combined close failed");
    assert_eq!(
        ipc::receive(nameservice, ns_endpoint),
        Err(IpcError::NoMessage),
        "cancelled combined call must not be delivered"
    );
    assert!(
        object::info(service, cancel_name).is_ok(),
        "copy source survives cancellation of the queued call"
    );
    object::close_cap(service, cancel_name).expect("cancel name close failed");
    object::close_cap(service, name_object).expect("named name close failed");

    close_user_address_space(service).expect("named service AS close failed");
    close_user_address_space(nameservice).expect("named ns AS close failed");
    logln!("Combined connection + copied-memory IPC attachment tests passed.");
}

pub fn test_vector_ipc_transaction_rollback() {
    logln!("Testing vector IPC transaction rollback...");
    let server = create_ipc_memory_test_address_space("vector-server");
    let client = create_ipc_memory_test_address_space("vector-client");
    let endpoint =
        ipc::endpoint_create(server, 0x5645_4354, 1, 4).expect("vector endpoint create failed");
    let connection = ipc::connection_delegate(server, endpoint, client, ConnectionRights::SEND)
        .expect("vector connection delegation failed");

    let moved = object::allocate(client, 1).expect("vector moved object allocation failed");
    let vector = object::allocate(client, 1).expect("vector page allocation failed");
    object::map(client, vector, VAddr::from(0x140000usize), true).expect("vector page map failed");
    let frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(client)
        .expect("vector client AS missing")
        .translate_address(VAddr::from(0x140000usize))
        .expect("vector page translation failed");
    let base: *mut u8 = frame.into();
    unsafe {
        core::ptr::write_volatile(base as *mut u16, 2);
        core::ptr::write_unaligned(
            base.add(2) as *mut ipc::CapVectorEntry,
            ipc::CapVectorEntry {
                cap: moved,
                mode: 1,
                _pad: 0,
            },
        );
        core::ptr::write_unaligned(
            base.add(2 + core::mem::size_of::<ipc::CapVectorEntry>()) as *mut ipc::CapVectorEntry,
            ipc::CapVectorEntry {
                cap: u64::MAX,
                mode: 0,
                _pad: 0,
            },
        );
    }
    object::unmap(client, vector).expect("vector page unmap failed");

    assert_eq!(
        ipc::vector_send(client, connection, 1, 0, vector),
        Err(IpcError::MemoryTransferFailed)
    );
    assert!(
        object::info(client, moved).is_ok(),
        "a failed vector must restore a moved cap under its original ID"
    );
    assert_eq!(
        ipc::receive(server, endpoint),
        Err(IpcError::NoMessage),
        "a partially transferred vector must not enqueue a message"
    );
    object::close_cap(client, moved).expect("vector moved object cleanup failed");
    object::close_cap(client, vector).expect("vector page cleanup failed");
    close_user_address_space(client).expect("vector client AS close failed");
    close_user_address_space(server).expect("vector server AS close failed");
    logln!("Vector IPC transaction rollback test passed.");
}
