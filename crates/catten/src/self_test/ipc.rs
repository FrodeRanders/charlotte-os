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
    assert_eq!(ipc::poll_reply(client, call), Ok(Some(-12)));
    assert_eq!(
        ipc::reply(server, reply, 13),
        Err(IpcError::UnknownCapability),
        "consumed reply token cap must not be reusable"
    );

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
        ipc::receive(client, endpoint),
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
