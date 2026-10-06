//! Kernel ABI fixtures deliberately use scalar IDs to inspect record admission
//! and cancellation. No userspace capability owner is duplicated here. Runs
//! before AP schedulers, when exact node-counter comparisons are stable.

use alloc::vec::Vec;

use super::*;

mod shared_admission;

fn account(asid: AddressSpaceId) -> Arc<record_budget::DomainBudget> {
    IPC.read().caps[&asid].record_budget.clone()
}

pub(crate) fn test_admission() {
    let baseline = record_budget::node_used();
    let server = crate::service::loader::create_user_address_space_handle();
    let client = crate::service::loader::create_user_address_space_handle();
    let endpoint = endpoint_create(server.id(), 1, 1, 1_024).unwrap();
    let connection =
        connection_delegate(server.id(), endpoint, client.id(), ConnectionRights::ALL).unwrap();
    let server_account = account(server.id());
    let client_account = {
        // Establish this namespace without introducing a sponsored connection.
        let mut ipc = IPC.write();
        ipc.as_caps(client.id()).record_budget.clone()
    };

    // Each minted/delegated capability, not just its endpoint, occupies a slot.
    // Re-delegation sponsors the new capability from the grantor's own budget.
    let mut connections = Vec::new();
    for _ in 1..record_budget::DOMAIN_LIMIT[0] {
        connections.push(
            connection_delegate(server.id(), endpoint, client.id(), ConnectionRights::ALL).unwrap(),
        );
    }
    assert_eq!(server_account.used(), [512, 0, 0]);
    assert_eq!(
        connection_mint(server.id(), endpoint, ConnectionRights::SEND),
        Err(IpcError::ResourceLimit)
    );
    let delegated =
        connection_delegate(client.id(), connection, server.id(), ConnectionRights::SEND).unwrap();
    assert_eq!(client_account.used(), [1, 0, 0]);
    close_cap(server.id(), delegated).unwrap();
    close_cap(client.id(), connections.pop().unwrap()).unwrap();
    let reused = connection_mint(server.id(), endpoint, ConnectionRights::SEND).unwrap();
    close_cap(server.id(), reused).unwrap();
    for cap in connections {
        close_cap(client.id(), cap).unwrap();
    }
    assert_eq!(server_account.used(), [1, 0, 0]);

    // Completed/observed calls remain charged until their owning cap closes.
    // This bounds callers retaining results even when the service queue is empty.
    let mut calls = Vec::new();
    for _ in 0..record_budget::DOMAIN_LIMIT[1] {
        let call = scalar_call(client.id(), connection, 1, 2).unwrap();
        let token = receive(server.id(), endpoint).unwrap().reply.unwrap();
        reply(server.id(), token, 42).unwrap();
        assert_eq!(poll_reply(client.id(), call).unwrap().unwrap().result, 42);
        calls.push(call);
    }
    assert_eq!(client_account.used(), [0, 512, 0]);
    assert_eq!(scalar_call(client.id(), connection, 1, 2), Err(IpcError::ResourceLimit));
    assert_eq!(endpoint_status(server.id(), endpoint).unwrap().1, 0);
    for cap in calls {
        close_cap(client.id(), cap).unwrap();
    }
    assert_eq!(client_account.used(), [0, 0, 0]);

    // Outstanding reply records are sponsored by the requesting caller. They
    // are already admitted while queued; receive needs no new record admission.
    let mut calls = Vec::new();
    for _ in 0..record_budget::DOMAIN_LIMIT[2] {
        calls.push(scalar_call(client.id(), connection, 1, 2).unwrap());
    }
    assert_eq!(client_account.used(), [0, 512, 512]);
    assert_eq!(scalar_call(client.id(), connection, 1, 2), Err(IpcError::ResourceLimit));
    close_cap(client.id(), calls.pop().unwrap()).unwrap();
    let replacement = scalar_call(client.id(), connection, 1, 2).unwrap();
    calls.push(replacement);
    for call in calls {
        close_cap(client.id(), call).unwrap();
    }
    assert_eq!(endpoint_status(server.id(), endpoint).unwrap().1, 0);
    assert_eq!(client_account.used(), [0, 0, 0]);

    let source = endpoint_create(client.id(), 2, 1, 4).unwrap();
    let memory = crate::memory::object::allocate(client.id(), 1).unwrap();
    let vector = crate::memory::object::allocate(client.id(), 1).unwrap();
    // One valid moved attachment. Submission rejection must not even begin it.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&memory.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    crate::memory::object::write_bytes(client.id(), vector, &bytes).unwrap();
    for dimension in [1, 2] {
        let mut amount = [0, 0, 0];
        amount[dimension] = record_budget::DOMAIN_LIMIT[dimension];
        let pressure = record_budget::reserve(&client_account, false, amount).unwrap();
        let node = record_budget::node_used();
        assert_eq!(scalar_call(client.id(), connection, 1, 2), Err(IpcError::ResourceLimit));
        assert_eq!(
            scalar_call_with_connection(
                client.id(),
                connection,
                1,
                2,
                source,
                ConnectionRights::SEND
            ),
            Err(IpcError::ResourceLimit)
        );
        assert_eq!(
            scalar_call_with_connection_copy(
                client.id(),
                connection,
                1,
                2,
                source,
                ConnectionRights::SEND,
                memory
            ),
            Err(IpcError::ResourceLimit)
        );
        assert_eq!(
            scalar_call_with_memory_move(client.id(), connection, 1, 2, memory),
            Err(IpcError::ResourceLimit)
        );
        assert_eq!(
            scalar_call_with_memory_copy(client.id(), connection, 1, 2, memory),
            Err(IpcError::ResourceLimit)
        );
        assert_eq!(
            scalar_call_with_memory_borrow_read(client.id(), connection, 1, 2, memory),
            Err(IpcError::ResourceLimit)
        );
        assert_eq!(
            scalar_call_with_memory_borrow_write(client.id(), connection, 1, 2, memory),
            Err(IpcError::ResourceLimit)
        );
        assert_eq!(
            vector_call(client.id(), connection, 1, 2, vector),
            Err(IpcError::ResourceLimit)
        );
        assert_eq!(endpoint_status(server.id(), endpoint).unwrap().1, 0);
        assert!(crate::memory::object::info(client.id(), memory).is_ok());
        assert!(crate::memory::object::info(client.id(), vector).is_ok());
        assert_eq!(client_account.used(), amount);
        assert_eq!(record_budget::node_used(), node);
        drop(pressure);
    }
    // Delegated connection admission also precedes copied attachment transfer.
    let pressure = record_budget::reserve(&client_account, false, [512, 0, 0]).unwrap();
    assert_eq!(
        scalar_call_with_connection_copy(
            client.id(),
            connection,
            1,
            2,
            source,
            ConnectionRights::SEND,
            memory
        ),
        Err(IpcError::ResourceLimit)
    );
    assert_eq!(client_account.used(), [512, 0, 0]);
    assert_eq!(endpoint_status(server.id(), endpoint).unwrap().1, 0);
    drop(pressure);
    // An attachment failure after staging must release both record reservations.
    assert_eq!(
        scalar_call_with_memory_copy(client.id(), connection, 1, 2, u64::MAX),
        Err(IpcError::MemoryTransferFailed)
    );
    assert_eq!(client_account.used(), [0, 0, 0]);

    // A grant failure on reply leaves both the token and delegated loan live.
    let call = scalar_call_with_memory_borrow_read(client.id(), connection, 1, 2, memory).unwrap();
    let message = receive(server.id(), endpoint).unwrap();
    let token = message.reply.unwrap();
    let loan = message.memory.unwrap();
    let pressure = record_budget::reserve(&client_account, false, [512, 0, 0]).unwrap();
    assert_eq!(
        reply_with_connection(server.id(), token, endpoint, ConnectionRights::SEND, 42),
        Err(IpcError::ResourceLimit)
    );
    assert!(crate::memory::object::info(server.id(), loan).is_ok());
    assert!(poll_reply(client.id(), call).unwrap().is_none());
    assert_eq!(client_account.used(), [512, 1, 1]);
    drop(pressure);
    reply_with_connection(server.id(), token, endpoint, ConnectionRights::SEND, 42).unwrap();
    assert!(crate::memory::object::info(server.id(), loan).is_err());
    assert_eq!(client_account.used(), [1, 1, 0]);
    assert_eq!(server_account.used(), [1, 0, 0]);
    // Unobserved results revoke the returned connection on pending-call close.
    close_cap(client.id(), call).unwrap();
    assert_eq!(client_account.used(), [0, 0, 0]);
    assert_eq!(server_account.used(), [1, 0, 0]);

    // Caller cancellation releases a queued delegated connection's sponsor.
    let call =
        scalar_call_with_connection(client.id(), connection, 1, 2, source, ConnectionRights::SEND)
            .unwrap();
    assert_eq!(client_account.used(), [1, 1, 1]);
    close_cap(client.id(), call).unwrap();
    assert_eq!(client_account.used(), [0, 0, 0]);
    // Observed returned authority outlives the pending call and closes once.
    let call = scalar_call(client.id(), connection, 1, 2).unwrap();
    let token = receive(server.id(), endpoint).unwrap().reply.unwrap();
    reply_with_connection(server.id(), token, endpoint, ConnectionRights::SEND, 42).unwrap();
    let returned = poll_reply(client.id(), call).unwrap().unwrap().cap.unwrap();
    close_cap(client.id(), call).unwrap();
    assert_eq!(client_account.used(), [1, 0, 0]);
    assert_eq!(server_account.used(), [1, 0, 0]);
    close_cap(client.id(), returned).unwrap();
    assert_eq!(client_account.used(), [0, 0, 0]);
    assert_eq!(server_account.used(), [1, 0, 0]);
    // Reproduce the IPC-only retirement interval before teardown takes its
    // capability snapshot. No receive/delegation may publish a late capability.
    let call = scalar_call(client.id(), connection, 1, 2).unwrap();
    server_account.retire();
    let node = record_budget::node_used();
    assert_eq!(receive(server.id(), endpoint), Err(IpcError::PermissionDenied));
    assert_eq!(receive_vec(server.id(), endpoint, 0), Err(IpcError::PermissionDenied));
    assert_eq!(
        connection_delegate(client.id(), connection, server.id(), ConnectionRights::SEND),
        Err(IpcError::PermissionDenied)
    );
    assert_eq!(
        scalar_call_with_connection(client.id(), connection, 1, 2, source, ConnectionRights::SEND),
        Err(IpcError::PermissionDenied)
    );
    assert_eq!(endpoint_status(server.id(), endpoint).unwrap().1, 1);
    assert_eq!(record_budget::node_used(), node);
    crate::memory::close_user_address_space_handle(server).unwrap();
    assert_eq!(poll_reply(client.id(), call).unwrap().unwrap().result, REPLY_ENDPOINT_CLOSED);
    close_cap(client.id(), call).unwrap();
    crate::memory::close_user_address_space_handle(client).unwrap();
    assert_eq!(server_account.used(), [0, 0, 0]);
    assert_eq!(client_account.used(), [0, 0, 0]);

    test_retirement();
    test_node_limits();
    shared_admission::test_admission();
    assert_eq!(record_budget::node_used(), baseline);
    crate::logln!(
        "[ipc records] SUCCESS: connection/call/reply bounds, pre-transfer rejection, rollback, \
         result/cancellation cleanup and generation reuse"
    );
}

fn test_retirement() {
    let recipient = 0x000e_b112;
    let old = crate::service::loader::create_user_address_space_handle();
    let endpoint = endpoint_create(old.id(), 1, 1, 4).unwrap();
    let connection =
        connection_delegate(old.id(), endpoint, recipient, ConnectionRights::SEND).unwrap();
    let old_account = account(old.id());
    crate::memory::budget::retire(old);
    assert_eq!(
        connection_mint(old.id(), endpoint, ConnectionRights::SEND),
        Err(IpcError::PermissionDenied)
    );
    crate::memory::close_user_address_space_handle(old).unwrap();
    assert_eq!(old_account.used(), [1, 0, 0]);
    assert!(record_budget::reserve(&old_account, false, [1, 0, 0]).is_err());
    let replacement = crate::service::loader::create_user_address_space_handle();
    assert_eq!(replacement.id(), old.id());
    assert_ne!(replacement, old);
    let endpoint = endpoint_create(replacement.id(), 1, 1, 4).unwrap();
    let fresh_connection =
        connection_mint(replacement.id(), endpoint, ConnectionRights::SEND).unwrap();
    let fresh = account(replacement.id());
    close_cap(recipient, connection).unwrap();
    assert_eq!(old_account.used(), [0, 0, 0]);
    assert_eq!(fresh.used(), [1, 0, 0]);
    close_cap(replacement.id(), fresh_connection).unwrap();
    crate::memory::close_user_address_space_handle(replacement).unwrap();
    close_address_space(recipient).unwrap();
}

fn test_node_limits() {
    // Counter saturation, not allocation of a maximum-footprint registry.
    for dimension in 0..3 {
        let baseline = record_budget::node_used();
        let mut charges = Vec::new();
        let mut remaining = record_budget::ORDINARY_LIMIT[dimension] - baseline.1[dimension];
        while remaining != 0 {
            let mut amount = [0, 0, 0];
            amount[dimension] = remaining.min(record_budget::DOMAIN_LIMIT[dimension]);
            charges.push(
                record_budget::reserve(&record_budget::DomainBudget::new(), false, amount).unwrap(),
            );
            remaining -= amount[dimension];
        }
        let rejected = record_budget::DomainBudget::new();
        let mut one = [0, 0, 0];
        one[dimension] = 1;
        let full = record_budget::node_used();
        assert!(record_budget::reserve(&rejected, false, one).is_err());
        assert_eq!(rejected.used(), [0, 0, 0]);
        assert_eq!(record_budget::node_used(), full);
        drop(record_budget::reserve(&rejected, true, one).unwrap());
        remaining = record_budget::NODE_LIMIT[dimension] - full.0[dimension];
        while remaining != 0 {
            let mut amount = [0, 0, 0];
            amount[dimension] = remaining.min(record_budget::DOMAIN_LIMIT[dimension]);
            charges.push(
                record_budget::reserve(&record_budget::DomainBudget::new(), true, amount).unwrap(),
            );
            remaining -= amount[dimension];
        }
        assert!(record_budget::reserve(&rejected, true, one).is_err());
        assert_eq!(rejected.used(), [0, 0, 0]);
        drop(charges);
        assert_eq!(record_budget::node_used(), baseline);
    }
}
