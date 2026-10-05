//! Real-domain kernel ABI fixtures. Raw IDs are intentional here: these tests
//! inspect registry admission, not duplicated userspace capability ownership.

use super::*;
use crate::{
    capability::admission_tests::{
        TEST_NAMESPACE_LIMIT,
        test_fill_remaining_namespace as fill,
        test_free_fixture_slot as free,
        test_namespace_used as used,
    },
    memory::{
        AddressSpaceHandle,
        object,
    },
};

struct Fixture {
    server: AddressSpaceHandle,
    client: AddressSpaceHandle,
    endpoint: u64,
    connection: u64,
    source: u64,
    result: u64,
}

impl Fixture {
    fn new() -> Self {
        let server = crate::service::loader::create_user_address_space_handle();
        let client = crate::service::loader::create_user_address_space_handle();
        let endpoint = endpoint_create(server.id(), 0x5155_4f54, 1, 4).unwrap();
        let connection =
            connection_delegate(server.id(), endpoint, client.id(), ConnectionRights::ALL).unwrap();
        let source = object::allocate(client.id(), 1).unwrap();
        let result = object::allocate(server.id(), 1).unwrap();
        object::write_bytes(server.id(), result, &[0xa5; 18]).unwrap();
        Self {
            server,
            client,
            endpoint,
            connection,
            source,
            result,
        }
    }

    fn call(&self) -> u64 {
        scalar_call_with_memory_borrow_read(self.client.id(), self.connection, 7, 42, self.source)
            .unwrap()
    }

    fn receive(&self, vector: bool) -> Result<ScalarMessage, IpcError> {
        if vector {
            receive_vec(self.server.id(), self.endpoint, self.result)
        } else {
            receive(self.server.id(), self.endpoint)
        }
    }

    fn assert_queued(&self, call: u64) {
        assert_eq!(endpoint_status(self.server.id(), self.endpoint).unwrap().1, 1);
        assert_eq!(poll_reply(self.client.id(), call).unwrap(), None);
        assert!(object::info(self.client.id(), self.source).unwrap().lent);
        assert_eq!(object::snapshot_bytes(self.server.id(), self.result, 18).unwrap(), [0xa5; 18]);
        assert_eq!(account(self.client.id()).used(), [0, 1, 1]);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        crate::memory::close_user_address_space_handle(self.client).unwrap();
        crate::memory::close_user_address_space_handle(self.server).unwrap();
        for handle in [self.client, self.server] {
            assert_eq!(
                crate::memory::budget::used(handle),
                crate::memory::budget::Amount::default()
            );
        }
    }
}

pub(super) fn test_admission() {
    let records = record_budget::node_used();
    let endpoints = budget::node_used();
    test_endpoint_and_connection();
    test_receive_quota();
    test_result_failure();
    test_receive_retirement();
    test_call_admission();
    test_joint_retirement();
    test_returned_connection();
    test_source_rejection();
    test_stale_call();
    assert_eq!(record_budget::node_used(), records);
    assert_eq!(budget::node_used(), endpoints);
    crate::logln!(
        "[ipc shared admission] endpoint/delegation refunds, scalar/vector receive pressure, \
         result failure, call/attachment composition, returned authority and retired publication \
         passed"
    );
}

fn test_endpoint_and_connection() {
    let f = Fixture::new();
    fill(f.server.id());
    fill(f.client.id());
    let metadata = endpoint_admission(f.server.id()).unwrap();
    let before = metadata.used();
    let records = record_budget::node_used();
    assert_eq!(endpoint_create(f.server.id(), 2, 1, 4), Err(IpcError::ResourceLimit));
    assert_eq!(metadata.used(), before, "failed identity admission refunds endpoint storage");
    assert_eq!(
        connection_mint(f.server.id(), f.endpoint, ConnectionRights::SEND),
        Err(IpcError::ResourceLimit)
    );
    assert_eq!(
        connection_delegate(f.server.id(), f.endpoint, f.client.id(), ConnectionRights::SEND),
        Err(IpcError::ResourceLimit)
    );
    assert_eq!(
        connection_delegate(f.client.id(), f.connection, f.server.id(), ConnectionRights::SEND),
        Err(IpcError::ResourceLimit)
    );
    assert_eq!(record_budget::node_used(), records, "failed recipient admission refunds grantor");
    assert_eq!(used(f.server.id()), TEST_NAMESPACE_LIMIT);
    assert_eq!(used(f.client.id()), TEST_NAMESPACE_LIMIT);
    free(f.client.id());
    let delegated =
        connection_delegate(f.server.id(), f.endpoint, f.client.id(), ConnectionRights::SEND)
            .unwrap();
    assert_eq!(used(f.client.id()), TEST_NAMESPACE_LIMIT);
    assert_eq!(account(f.server.id()).used(), [2, 0, 0]);
    close_cap(f.client.id(), delegated).unwrap();
    assert_eq!(account(f.server.id()).used(), [1, 0, 0]);
    free(f.server.id());
    let endpoint = endpoint_create(f.server.id(), 2, 1, 4).unwrap();
    close_cap(f.server.id(), endpoint).unwrap();
    assert_eq!(metadata.used(), before);
}

fn test_receive_quota() {
    for vector in [false, true] {
        for cancel in [false, true] {
            let f = Fixture::new();
            let call = f.call();
            // Attachments already occupy receiver slots. Reply-cap publication
            // needs its own shared slot, but no new caller-sponsored token record.
            fill(f.server.id());
            let records = record_budget::node_used();
            for _ in 0..3 {
                assert_eq!(f.receive(vector), Err(IpcError::ResourceLimit));
                f.assert_queued(call);
                assert_eq!(used(f.server.id()), TEST_NAMESPACE_LIMIT);
                assert_eq!(record_budget::node_used(), records);
            }
            if cancel {
                close_cap(f.client.id(), call).unwrap();
                assert_eq!(f.receive(vector), Err(IpcError::NoMessage));
                assert!(!object::info(f.client.id(), f.source).unwrap().lent);
                assert_eq!(used(f.server.id()), TEST_NAMESPACE_LIMIT - 1);
                assert_eq!(account(f.client.id()).used(), [0, 0, 0]);
            } else {
                free(f.server.id());
                let message = f.receive(vector).unwrap();
                assert_eq!(message.opcode, 7);
                assert_eq!(message.arg0, 42);
                let borrowed = message.memory.unwrap();
                if vector {
                    let bytes = object::snapshot_bytes(f.server.id(), f.result, 10).unwrap();
                    assert_eq!(&bytes[..2], &1u16.to_le_bytes());
                    assert_eq!(&bytes[2..], &borrowed.to_le_bytes());
                }
                assert_eq!(used(f.server.id()), TEST_NAMESPACE_LIMIT);
                assert_eq!(f.receive(vector), Err(IpcError::NoMessage));
                reply(f.server.id(), message.reply.unwrap(), 99).unwrap();
                assert_eq!(poll_reply(f.client.id(), call).unwrap().unwrap().result, 99);
                assert!(!object::info(f.client.id(), f.source).unwrap().lent);
                assert_eq!(used(f.server.id()), TEST_NAMESPACE_LIMIT - 2);
                close_cap(f.client.id(), call).unwrap();
            }
        }
        // One-way messages need no reply slot and can drain a full namespace.
        let f = Fixture::new();
        scalar_send(f.client.id(), f.connection, 8, 0).unwrap();
        fill(f.server.id());
        assert_eq!(f.receive(vector).unwrap().reply, None);
        assert_eq!(used(f.server.id()), TEST_NAMESPACE_LIMIT);
    }
}

fn test_result_failure() {
    let f = Fixture::new();
    let call = f.call();
    let before = used(f.server.id());
    let records = record_budget::node_used();
    // Invalid output must return the speculative slot without consuming the
    // reply token or revoking the queued loan, even across repeated retries.
    for _ in 0..3 {
        assert_eq!(
            receive_vec(f.server.id(), f.endpoint, u64::MAX),
            Err(IpcError::MemoryTransferFailed)
        );
        f.assert_queued(call);
        assert_eq!(used(f.server.id()), before);
        assert_eq!(record_budget::node_used(), records);
    }
    // A read-loaned result source is also unwritable. Its existing bytes and
    // both independent loans survive the failed vector receive.
    let borrowed_result = object::lend_read(f.server.id(), f.result, f.client.id()).unwrap();
    assert_eq!(f.receive(true), Err(IpcError::MemoryTransferFailed));
    f.assert_queued(call);
    assert_eq!(used(f.server.id()), before);
    object::revoke_lend(f.server.id(), f.result, f.client.id(), borrowed_result).unwrap();
    let message = f.receive(true).unwrap();
    reply(f.server.id(), message.reply.unwrap(), 99).unwrap();
    close_cap(f.client.id(), call).unwrap();
}

fn test_receive_retirement() {
    let f = Fixture::new();
    let call = f.call();
    let count = used(f.server.id());
    // Pause at the exact reservation/publication boundary under IPC. Retiring
    // CAP admission must reject without any payload/queue/result mutation.
    let mut ipc = IPC.write();
    let reservation = ipc.reserve_cap(f.server.id()).unwrap();
    crate::capability::retire_address_space(f.server.id());
    assert_eq!(reservation.publish(), Err(crate::capability::AllocationError::Retired));
    assert_eq!(used(f.server.id()), count);
    drop(ipc);
    for vector in [false, true] {
        assert_eq!(f.receive(vector), Err(IpcError::PermissionDenied));
        f.assert_queued(call);
    }
    close_cap(f.client.id(), call).unwrap();
    assert!(!object::info(f.client.id(), f.source).unwrap().lent);
}

fn invoke_call(f: &Fixture, variant: usize, descriptor: u64) -> Result<u64, IpcError> {
    let caller = f.client.id();
    match variant {
        0 => scalar_call(caller, f.connection, 1, 0),
        1 => scalar_call_with_connection(
            caller,
            f.connection,
            1,
            0,
            f.connection,
            ConnectionRights::SEND,
        ),
        2 => scalar_call_with_connection_copy(
            caller,
            f.connection,
            1,
            0,
            f.connection,
            ConnectionRights::SEND,
            f.source,
        ),
        3 => scalar_call_with_memory_move(caller, f.connection, 1, 0, f.source),
        4 => scalar_call_with_memory_copy(caller, f.connection, 1, 0, f.source),
        5 => scalar_call_with_memory_borrow_read(caller, f.connection, 1, 0, f.source),
        6 => scalar_call_with_memory_borrow_write(caller, f.connection, 1, 0, f.source),
        7 => vector_call(caller, f.connection, 1, 0, descriptor),
        _ => unreachable!(),
    }
}

fn test_call_admission() {
    let f = Fixture::new();
    let descriptor = object::allocate(f.client.id(), 1).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&f.source.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    object::write_bytes(f.client.id(), descriptor, &bytes).unwrap();
    let backing = crate::memory::budget::used(f.client);
    let target = used(f.server.id());
    let records = record_budget::node_used();
    fill(f.client.id());
    for variant in 0..8 {
        assert_eq!(invoke_call(&f, variant, descriptor), Err(IpcError::ResourceLimit));
        assert_eq!(used(f.client.id()), TEST_NAMESPACE_LIMIT);
        assert_eq!(used(f.server.id()), target);
        assert_eq!(endpoint_status(f.server.id(), f.endpoint).unwrap().1, 0);
        assert_eq!(record_budget::node_used(), records);
        assert_eq!(crate::memory::budget::used(f.client), backing);
        assert!(!object::info(f.client.id(), f.source).unwrap().lent);
        object::write_bytes(f.client.id(), f.source, &[0x5a]).unwrap();
        assert!(object::info(f.client.id(), descriptor).is_ok());
    }
    free(f.client.id());
    fill(f.server.id());
    // The source has exactly one slot for staged call authority. A denied
    // attachment must return that slot and both caller-sponsored records.
    for variant in 1..8 {
        let expected = if variant <= 2 {
            IpcError::ResourceLimit
        } else {
            IpcError::MemoryTransferFailed
        };
        assert_eq!(invoke_call(&f, variant, descriptor), Err(expected));
        assert_eq!(used(f.client.id()), TEST_NAMESPACE_LIMIT - 1);
        assert_eq!(used(f.server.id()), TEST_NAMESPACE_LIMIT);
        assert_eq!(record_budget::node_used(), records);
        assert_eq!(crate::memory::budget::used(f.client), backing);
        assert!(!object::info(f.client.id(), f.source).unwrap().lent);
        object::write_bytes(f.client.id(), f.source, &[0xa5]).unwrap();
        assert!(object::info(f.client.id(), descriptor).is_ok());
        assert_eq!(endpoint_status(f.server.id(), f.endpoint).unwrap().1, 0);
    }
    // Connection plus copied memory needs two receiver slots. With only one,
    // the staged connection must not escape when copy admission fails.
    free(f.server.id());
    assert_eq!(invoke_call(&f, 2, descriptor), Err(IpcError::MemoryTransferFailed));
    assert_eq!(used(f.client.id()), TEST_NAMESPACE_LIMIT - 1);
    assert_eq!(used(f.server.id()), TEST_NAMESPACE_LIMIT - 1);
    assert_eq!(record_budget::node_used(), records);
    assert_eq!(crate::memory::budget::used(f.client), backing);
    free(f.server.id());
    let call = invoke_call(&f, 2, descriptor).unwrap();
    assert_eq!(used(f.client.id()), TEST_NAMESPACE_LIMIT);
    assert_eq!(used(f.server.id()), TEST_NAMESPACE_LIMIT);
    assert_eq!(account(f.client.id()).used(), [1, 1, 1]);
    assert_eq!(receive(f.server.id(), f.endpoint), Err(IpcError::ResourceLimit));
    free(f.server.id());
    let message = receive(f.server.id(), f.endpoint).unwrap();
    reply(f.server.id(), message.reply.unwrap(), 42).unwrap();
    close_cap(f.server.id(), message.connection.unwrap()).unwrap();
    object::close_cap(f.server.id(), message.memory.unwrap()).unwrap();
    close_cap(f.client.id(), call).unwrap();
    assert_eq!(record_budget::node_used(), records);
    assert_eq!(crate::memory::budget::used(f.client), backing);
}

fn test_joint_retirement() {
    for retire_caller in [false, true] {
        for mixed in [false, true] {
            let f = Fixture::new();
            let sources =
                core::array::from_fn::<_, 4, _>(|_| object::allocate(f.client.id(), 1).unwrap());
            let before = crate::memory::budget::used(f.client);
            let source_count = used(f.client.id());
            let target_count = used(f.server.id());
            let records = record_budget::node_used();
            let mut ipc = IPC.write();
            let mut call = ipc.stage_call(f.client.id()).unwrap();
            let staged_call = call.authority.identity();
            call.connection = Some(
                PreparedConnection::new(
                    &mut ipc,
                    f.client.id(),
                    f.server.id(),
                    f.endpoint,
                    ConnectionRights::SEND,
                )
                .unwrap(),
            );
            let staged_connection = call.connection.as_ref().unwrap().authority.identity();
            call.attach(object::prepare_copy(f.client.id(), sources[0], f.server.id()).unwrap())
                .unwrap();
            if mixed {
                call.attach(
                    object::prepare_move(f.client.id(), sources[1], f.server.id()).unwrap(),
                )
                .unwrap();
                call.attach(
                    object::prepare_loan(f.client.id(), sources[2], f.server.id(), false).unwrap(),
                )
                .unwrap();
                call.attach(
                    object::prepare_loan(f.client.id(), sources[3], f.server.id(), true).unwrap(),
                )
                .unwrap();
            }
            let staged_memory = call.memory.clone();
            assert_eq!(ipc.cap(f.client.id(), staged_call), Err(IpcError::UnknownCapability));
            assert_eq!(ipc.cap(f.server.id(), staged_connection), Err(IpcError::UnknownCapability));
            for &cap in &staged_memory {
                assert_eq!(
                    object::info(f.server.id(), cap),
                    Err(object::MemoryObjectError::UnknownCapability)
                );
            }
            // Copy-only caller retirement specifically requires the *additional
            // IPC identity* to participate: copies have no source escrow entry.
            crate::capability::retire_address_space(
                if retire_caller {
                    f.client.id()
                } else {
                    f.server.id()
                },
            );
            assert!(matches!(
                call.commit(&mut ipc, f.endpoint, 1, 0),
                Err(IpcError::MemoryTransferFailed)
            ));
            assert_eq!(ipc.cap(f.client.id(), staged_call), Err(IpcError::UnknownCapability));
            assert_eq!(ipc.cap(f.server.id(), staged_connection), Err(IpcError::UnknownCapability));
            drop(ipc);
            for cap in staged_memory {
                assert_eq!(
                    object::info(f.server.id(), cap),
                    Err(object::MemoryObjectError::UnknownCapability)
                );
            }
            assert_eq!(used(f.client.id()), source_count);
            assert_eq!(used(f.server.id()), target_count);
            assert_eq!(record_budget::node_used(), records);
            assert_eq!(crate::memory::budget::used(f.client), before);
            assert_eq!(endpoint_status(f.server.id(), f.endpoint).unwrap().1, 0);
            for cap in sources {
                assert!(!object::info(f.client.id(), cap).unwrap().lent);
                object::write_bytes(f.client.id(), cap, &[0x5a]).unwrap();
            }
        }
    }
}

fn test_returned_connection() {
    let f = Fixture::new();
    let call = f.call();
    let message = receive(f.server.id(), f.endpoint).unwrap();
    let reply_cap = message.reply.unwrap();
    let borrowed = message.memory.unwrap();
    fill(f.client.id());
    let records = record_budget::node_used();
    assert_eq!(
        reply_with_connection(f.server.id(), reply_cap, f.endpoint, ConnectionRights::SEND, 42),
        Err(IpcError::ResourceLimit)
    );
    assert_eq!(record_budget::node_used(), records);
    assert_eq!(account(f.client.id()).used(), [0, 1, 1]);
    assert!(object::info(f.client.id(), f.source).unwrap().lent);
    assert!(object::info(f.server.id(), borrowed).is_ok());
    assert_eq!(poll_reply(f.client.id(), call).unwrap(), None);
    free(f.client.id());
    reply_with_connection(f.server.id(), reply_cap, f.endpoint, ConnectionRights::SEND, 42)
        .unwrap();
    let returned = poll_reply(f.client.id(), call).unwrap().unwrap().cap.unwrap();
    assert_eq!(used(f.client.id()), TEST_NAMESPACE_LIMIT);
    assert!(!object::info(f.client.id(), f.source).unwrap().lent);
    assert_eq!(
        object::info(f.server.id(), borrowed),
        Err(object::MemoryObjectError::UnknownCapability)
    );
    close_cap(f.client.id(), call).unwrap();
    assert_eq!(account(f.client.id()).used(), [1, 0, 0]);
    scalar_send(f.client.id(), returned, 99, 0).unwrap();
    assert_eq!(receive(f.server.id(), f.endpoint).unwrap().opcode, 99);
    close_cap(f.client.id(), returned).unwrap();
    assert_eq!(account(f.client.id()).used(), [0, 0, 0]);
}

fn test_source_rejection() {
    let f = Fixture::new();
    let records = record_budget::node_used();
    let source_count = used(f.client.id());
    let target_count = used(f.server.id());
    assert_eq!(
        scalar_call_with_connection_copy(
            f.client.id(),
            f.connection,
            1,
            0,
            f.connection,
            ConnectionRights::SEND,
            u64::MAX
        ),
        Err(IpcError::MemoryTransferFailed)
    );
    // An existing source mapping rejects move/write-loan preparation. No call
    // slot, family charge or destination authority may escape either failure.
    object::map_any(f.client.id(), f.source, false).unwrap();
    for variant in [3, 6] {
        assert_eq!(invoke_call(&f, variant, 0), Err(IpcError::MemoryTransferFailed));
        assert!(!object::info(f.client.id(), f.source).unwrap().lent);
        assert_eq!(used(f.client.id()), source_count);
        assert_eq!(used(f.server.id()), target_count);
        assert_eq!(record_budget::node_used(), records);
        assert_eq!(endpoint_status(f.server.id(), f.endpoint).unwrap().1, 0);
    }
    object::unmap(f.client.id(), f.source).unwrap();
}

fn test_stale_call() {
    let server = crate::service::loader::create_user_address_space_handle();
    let old = crate::service::loader::create_user_address_space_handle();
    let endpoint = endpoint_create(server.id(), 1, 1, 4).unwrap();
    connection_delegate(server.id(), endpoint, old.id(), ConnectionRights::CALL).unwrap();
    let source = object::allocate(old.id(), 1).unwrap();
    let mut ipc = IPC.write();
    let mut stale = ipc.stage_call(old.id()).unwrap();
    let old_cap = stale.authority.identity();
    stale.connection = Some(
        PreparedConnection::new(&mut ipc, old.id(), server.id(), endpoint, ConnectionRights::SEND)
            .unwrap(),
    );
    stale.attach(object::prepare_copy(old.id(), source, server.id()).unwrap()).unwrap();
    let old_records = ipc.as_caps(old.id()).record_budget.clone();
    drop(ipc);
    crate::memory::close_user_address_space_handle(old).unwrap();
    assert_eq!(old_records.used(), [1, 1, 1]);
    assert_eq!(
        crate::memory::budget::used(old),
        crate::memory::budget::Amount {
            pages: 1,
            objects: 1
        }
    );
    let fresh = crate::service::loader::create_user_address_space_handle();
    assert_eq!(fresh.id(), old.id());
    assert_ne!(fresh, old);
    let fresh_endpoint = endpoint_create(fresh.id(), 2, 1, 4).unwrap();
    let connection = connection_mint(fresh.id(), fresh_endpoint, ConnectionRights::CALL).unwrap();
    let call = scalar_call(fresh.id(), connection, 1, 0).unwrap();
    assert_eq!(call, old_cap, "reuse the exact staged IPC handle in the successor");
    let fresh_records = account(fresh.id());
    let counts = used(fresh.id());
    let mut ipc = IPC.write();
    assert!(matches!(stale.commit(&mut ipc, endpoint, 1, 0), Err(IpcError::MemoryTransferFailed)));
    assert!(matches!(ipc.cap(fresh.id(), call), Ok(Capability::PendingCall { .. })));
    drop(ipc);
    assert_eq!(old_records.used(), [0, 0, 0]);
    assert_eq!(crate::memory::budget::used(old), crate::memory::budget::Amount::default());
    assert_eq!(crate::memory::budget::used(fresh), crate::memory::budget::Amount::default());
    assert_eq!(fresh_records.used(), [1, 1, 1]);
    assert_eq!(used(fresh.id()), counts);
    assert_eq!(endpoint_status(server.id(), endpoint).unwrap().1, 0);
    let token = receive(fresh.id(), fresh_endpoint).unwrap().reply.unwrap();
    reply(fresh.id(), token, 42).unwrap();
    assert_eq!(poll_reply(fresh.id(), call).unwrap().unwrap().result, 42);
    crate::memory::close_user_address_space_handle(fresh).unwrap();
    crate::memory::close_user_address_space_handle(server).unwrap();
}
