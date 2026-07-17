//! Adversarial IPC and capability tests (success criterion 12).
//!
//! Every capability and memory-transfer feature requires negative tests per
//! the architecture doc S18.4. These tests operate at the kernel API level
//! (no EL0 domains) and cover the IPC error paths not exercised by the
//! positive happy-path tests.
use crate::{
    cpu::isa::interface::memory::AddressSpaceInterface,
    ipc::{
        self,
        ConnectionRights,
        IpcError,
    },
    logln,
    memory,
};

const ASV_A: usize = 0x0ad1;
const ASV_B: usize = 0x0ad2;
const ASV_C: usize = 0x0ad3;

pub fn test_adversarial_ipc() {
    // Address spaces for isolation testing
    memory::ADDRESS_SPACE_TABLE.lock().add_element(
        memory::AddressSpace::get_current()
    );
    memory::ADDRESS_SPACE_TABLE.lock().add_element(
        memory::AddressSpace::get_current()
    );
    memory::ADDRESS_SPACE_TABLE.lock().add_element(
        memory::AddressSpace::get_current()
    );

    logln!("Testing adversarial IPC scenarios...");

    test_double_close();
    test_double_reply();
    test_wrong_asid();
    test_insufficient_rights();
    test_queue_full();
    test_cancellation_race();

    logln!("Adversarial IPC tests passed.");
}

fn test_double_close() {
    // Create an endpoint, mint a connection, close it twice.
    let endpoint = ipc::endpoint_create(ASV_A, 0xabcd, 1, 4).unwrap();
    let conn = ipc::connection_mint(ASV_A, endpoint, ConnectionRights::SEND | ConnectionRights::CALL).unwrap();

    // First close succeeds.
    assert_eq!(ipc::close_cap(ASV_A, conn), Ok(()), "first close must succeed");

    // Second close must fail — cap no longer exists.
    assert_eq!(
        ipc::close_cap(ASV_A, conn),
        Err(IpcError::UnknownCapability),
        "closing an already-closed cap must fail"
    );

    // Clean up the endpoint.
    ipc::close_cap(ASV_A, endpoint).unwrap();
    logln!("[adversarial] double-close rejected");
}

fn test_double_reply() {
    // A server replies to a call, then tries to reply again — must fail.
    let endpoint = ipc::endpoint_create(ASV_A, 0xef01, 1, 4).unwrap();
    let conn = ipc::connection_mint(ASV_A, endpoint, ConnectionRights::SEND | ConnectionRights::CALL).unwrap();
    let conn2 = ipc::connection_delegate(ASV_A, endpoint, ASV_B, ConnectionRights::CALL).unwrap();

    // Client calls the server.
    let call = ipc::scalar_call(ASV_B, conn2, 1, 42).unwrap();

    // Server receives.
    let msg = ipc::receive(ASV_A, endpoint).unwrap();
    let reply_token = msg.reply.unwrap();

    // First reply succeeds.
    assert_eq!(ipc::reply(ASV_A, reply_token, 99), Ok(()));

    // Second reply must fail — token already consumed.
    assert_eq!(
        ipc::reply(ASV_A, reply_token, 100),
        Err(IpcError::UnknownCapability),
        "replying with an already-used token must fail"
    );

    // Client should see the first reply (value 99).
    if let Ok(Some(reply)) = ipc::poll_reply(ASV_B, call) {
        assert_eq!(reply.result, 99);
    }

    ipc::close_cap(ASV_A, conn).unwrap();
    ipc::close_cap(ASV_B, call).unwrap();
    ipc::close_cap(ASV_A, endpoint).unwrap();
    logln!("[adversarial] double-reply rejected");
}

fn test_wrong_asid() {
    // Capabilities are per-address-space. Using a cap in the wrong AS fails.
    let endpoint = ipc::endpoint_create(ASV_A, 0xcafe, 1, 4).unwrap();
    let conn = ipc::connection_mint(ASV_A, endpoint, ConnectionRights::SEND).unwrap();

    // ASV_B tries to use ASV_A's cap directly — must fail.
    assert_eq!(
        ipc::scalar_send(ASV_B, conn, 7, 0),
        Err(IpcError::UnknownCapability),
        "using another AS's cap must fail"
    );

    // But proper delegation (connection_mint in the target AS) should work.
    let delegated = ipc::connection_delegate(ASV_A, endpoint, ASV_C, ConnectionRights::SEND).unwrap();
    assert_eq!(ipc::scalar_send(ASV_C, delegated, 7, 0), Ok(()));

    ipc::close_cap(ASV_A, conn).unwrap();
    ipc::close_cap(ASV_C, delegated).unwrap();
    ipc::close_cap(ASV_A, endpoint).unwrap();
    logln!("[adversarial] wrong-ASID rejected, delegation accepted");
}

fn test_insufficient_rights() {
    // A connection with SEND-only rights must reject CALL.
    let endpoint = ipc::endpoint_create(ASV_A, 0xf00d, 1, 4).unwrap();
    let conn = ipc::connection_mint(ASV_A, endpoint, ConnectionRights::SEND).unwrap();
    let conn_full = ipc::connection_mint(ASV_A, endpoint, ConnectionRights::SEND | ConnectionRights::CALL).unwrap();

    // SEND should work on both.
    assert_eq!(ipc::scalar_send(ASV_A, conn, 1, 0), Ok(()));
    assert_eq!(ipc::scalar_send(ASV_A, conn_full, 2, 0), Ok(()));

    // CALL must fail on SEND-only connection.
    assert_eq!(
        ipc::scalar_call(ASV_A, conn, 3, 0),
        Err(IpcError::PermissionDenied),
        "CALL on a SEND-only connection must fail"
    );

    // CALL must succeed on full-rights connection.
    let call = ipc::scalar_call(ASV_A, conn_full, 4, 0).unwrap();
    // Drain the queued messages: two SENDs (no reply token) then the CALL.
    for _ in 0..2 {
        let m = ipc::receive(ASV_A, endpoint).unwrap();
        assert!(m.reply.is_none(), "SEND messages carry no reply token");
    }
    let last = ipc::receive(ASV_A, endpoint).unwrap();
    ipc::reply(ASV_A, last.reply.unwrap(), 0).unwrap();
    ipc::close_cap(ASV_A, call).unwrap();

    ipc::close_cap(ASV_A, conn).unwrap();
    ipc::close_cap(ASV_A, conn_full).unwrap();
    ipc::close_cap(ASV_A, endpoint).unwrap();
    logln!("[adversarial] insufficient-rights rejected");
}

fn test_queue_full() {
    // Fill a server's queue to capacity; the next send must fail.
    let capacity: usize = 2;
    let endpoint = ipc::endpoint_create(ASV_A, 0xbaad, 1, capacity).unwrap();
    let conn = ipc::connection_mint(ASV_A, endpoint, ConnectionRights::SEND).unwrap();

    // Fill the queue.
    assert_eq!(ipc::scalar_send(ASV_A, conn, 1, 10), Ok(()));
    assert_eq!(ipc::scalar_send(ASV_A, conn, 1, 20), Ok(()));

    // One more must fail.
    assert_eq!(
        ipc::scalar_send(ASV_A, conn, 1, 30),
        Err(IpcError::QueueFull),
        "send to a full queue must fail"
    );

    // Drain and verify the first two arrived in order.
    let m1 = ipc::receive(ASV_A, endpoint).unwrap();
    assert_eq!(m1.arg0, 10);
    let m2 = ipc::receive(ASV_A, endpoint).unwrap();
    assert_eq!(m2.arg0, 20);
    assert_eq!(ipc::receive(ASV_A, endpoint), Err(IpcError::NoMessage));

    ipc::close_cap(ASV_A, conn).unwrap();
    ipc::close_cap(ASV_A, endpoint).unwrap();
    logln!("[adversarial] queue-full rejected, ordering preserved");
}

fn test_cancellation_race() {
    // Cancel a pending call, then have the server try to reply — must fail.
    let endpoint = ipc::endpoint_create(ASV_A, 0xdead, 1, 4).unwrap();
    let conn = ipc::connection_delegate(ASV_A, endpoint, ASV_B, ConnectionRights::CALL).unwrap();

    // Client calls, then immediately cancels before server processes.
    let call = ipc::scalar_call(ASV_B, conn, 1, 99).unwrap();

    // Cancel the call while it's queued (not yet received by server).
    ipc::close_cap(ASV_B, call).unwrap();

    // Server tries to receive — the cancelled call is removed from queue.
    assert_eq!(
        ipc::receive(ASV_A, endpoint),
        Err(IpcError::NoMessage),
        "cancelled queued call must not be delivered"
    );

    ipc::close_cap(ASV_B, conn).unwrap();
    ipc::close_cap(ASV_A, endpoint).unwrap();
    logln!("[adversarial] queued-call cancellation clears the queue");
}
