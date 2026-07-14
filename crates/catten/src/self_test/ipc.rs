//! Self-tests for endpoint IPC.

use crate::{
    ipc::{
        self,
        ConnectionRights,
        IpcError,
    },
    logln,
};

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

    ipc::close_address_space(client);
    ipc::close_address_space(server);
    logln!("Endpoint IPC subsystem tests passed.");
}
