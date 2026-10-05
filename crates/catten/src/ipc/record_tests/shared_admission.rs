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
    assert_eq!(record_budget::node_used(), records);
    assert_eq!(budget::node_used(), endpoints);
    crate::logln!(
        "[ipc shared admission] endpoint/delegation refunds, scalar/vector receive pressure, \
         result failure, retry/cancellation and retired publication passed"
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
