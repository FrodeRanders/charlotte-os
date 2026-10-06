//! Raw kernel ABI fixtures for guessed, undelivered owning IPC capabilities.
//! These scalars inspect admitted IPC records; applications use runtime owners.
mod backing;
mod connection;
use super::*;
use crate::memory::{
    self,
    AddressSpaceHandle,
    object::{
        self,
        MemoryObjectError,
    },
};

struct Fixture {
    caller: AddressSpaceHandle,
    server: AddressSpaceHandle,
    endpoint: CapabilityId,
    connection: CapabilityId,
}

impl Fixture {
    fn new() -> Self {
        let caller = crate::service::loader::create_user_address_space_handle();
        let server = crate::service::loader::create_user_address_space_handle();
        let endpoint = endpoint_create(server.id(), 0x4445_4c49, 1, 4).unwrap();
        let connection =
            connection_delegate(server.id(), endpoint, caller.id(), ConnectionRights::ALL).unwrap();
        Self {
            caller,
            server,
            endpoint,
            connection,
        }
    }

    fn queued(&self) -> Vec<MemoryObjectCap> {
        let ipc = IPC.read();
        let Capability::Endpoint {
            endpoint,
            ..
        } = ipc.cap(self.server.id(), self.endpoint).unwrap()
        else {
            unreachable!()
        };
        ipc.endpoints[&endpoint].queue.front().unwrap().memory.to_vec()
    }

    fn close(self) {
        memory::close_user_address_space_handle(self.caller).unwrap();
        memory::close_user_address_space_handle(self.server).unwrap();
    }
}

fn hidden(asid: AddressSpaceId, cap: MemoryObjectCap, peer: AddressSpaceId) {
    // Presence proves this is a live but undelivered payload, not a bad guess.
    assert!(crate::capability::contains(asid, cap, crate::capability::ObjectKind::Memory));
    assert_eq!(object::info(asid, cap), Err(MemoryObjectError::UnknownCapability));
    assert_eq!(object::snapshot_bytes(asid, cap, 1), Err(MemoryObjectError::UnknownCapability));
    assert_eq!(object::write_bytes(asid, cap, &[9]), Err(MemoryObjectError::UnknownCapability));
    assert_eq!(object::map_any(asid, cap, true), Err(MemoryObjectError::UnknownCapability));
    assert_eq!(object::unmap(asid, cap), Err(MemoryObjectError::UnknownCapability));
    assert_eq!(object::close_cap(asid, cap), Err(MemoryObjectError::UnknownCapability));
    assert!(matches!(
        object::prepare_move(asid, cap, peer),
        Err(MemoryObjectError::UnknownCapability)
    ));
    assert!(matches!(
        object::prepare_copy(asid, cap, peer),
        Err(MemoryObjectError::UnknownCapability)
    ));
    assert!(matches!(
        object::prepare_loan(asid, cap, peer, false),
        Err(MemoryObjectError::UnknownCapability)
    ));
    assert!(matches!(
        object::pin_for_dma(asid, cap, true, false, false),
        Err(MemoryObjectError::UnknownCapability)
    ));
    assert!(crate::capability::contains(asid, cap, crate::capability::ObjectKind::Memory));
}

fn reclaimed(asid: AddressSpaceId, cap: MemoryObjectCap) {
    assert!(!crate::capability::contains(asid, cap, crate::capability::ObjectKind::Memory));
}

pub(crate) fn run() {
    connection::run();
    backing::run();
    submission::tests::run();
    queued_scalar();
    queued_vector();
    returned_memory();
    crate::logln!(
        "[IPC memory delivery] guessed move/copy/result use rejected; receive rollback, vector \
         delivery, cancellation, endpoint/root cleanup and observed ownership passed"
    );
}

fn queued_scalar() {
    // Calls and asynchronous sends, copies and moves, delivered and cancelled.
    for copy in [false, true] {
        for call in [false, true] {
            for deliver in [false, true] {
                let fixture = Fixture::new();
                let source = object::allocate(fixture.caller.id(), 1).unwrap();
                object::write_bytes(fixture.caller.id(), source, &[0xa5]).unwrap();
                let pending = if call {
                    Some(
                        if copy {
                            scalar_call_with_memory_copy
                        } else {
                            scalar_call_with_memory_move
                        }(
                            fixture.caller.id(), fixture.connection, 1, 0, source
                        )
                        .unwrap(),
                    )
                } else {
                    let send = if copy {
                        scalar_send_with_memory_copy
                    } else {
                        scalar_send_with_memory_move
                    };
                    send(fixture.caller.id(), fixture.connection, 1, 0, source).unwrap();
                    None
                };
                let destination = fixture.queued()[0];
                hidden(fixture.server.id(), destination, fixture.caller.id());
                // A guessed attachment is also an invalid vector-result page.
                assert_eq!(
                    receive_vec(fixture.server.id(), fixture.endpoint, destination),
                    Err(IpcError::MemoryTransferFailed)
                );
                assert_eq!(fixture.queued(), [destination]);
                hidden(fixture.server.id(), destination, fixture.caller.id());
                if deliver {
                    let message = receive(fixture.server.id(), fixture.endpoint).unwrap();
                    assert_eq!(message.memory, Some(destination));
                    assert_eq!(
                        object::snapshot_bytes(fixture.server.id(), destination, 1).unwrap(),
                        [0xa5]
                    );
                    object::map_any(fixture.server.id(), destination, true).unwrap();
                    if let Some(pending) = pending {
                        close_cap(fixture.caller.id(), pending).unwrap();
                    }
                    close_cap(fixture.server.id(), fixture.endpoint).unwrap();
                    // Delivery transfers ownership: cancellation must leave it mapped.
                    assert!(object::info(fixture.server.id(), destination).unwrap().mapped);
                    object::unmap(fixture.server.id(), destination).unwrap();
                    object::close_cap(fixture.server.id(), destination).unwrap();
                } else {
                    if let Some(pending) = pending {
                        close_cap(fixture.caller.id(), pending).unwrap();
                    }
                    close_cap(fixture.server.id(), fixture.endpoint).unwrap();
                    reclaimed(fixture.server.id(), destination);
                }
                if copy {
                    object::close_cap(fixture.caller.id(), source).unwrap();
                }
                assert_eq!(memory::budget::used(fixture.caller), memory::budget::Amount::default());
                fixture.close();
            }
        }
    }
    // Whole-server teardown also reclaims hidden authority and donor backing.
    let fixture = Fixture::new();
    let source = object::allocate(fixture.caller.id(), 1).unwrap();
    let pending =
        scalar_call_with_memory_move(fixture.caller.id(), fixture.connection, 1, 0, source)
            .unwrap();
    let destination = fixture.queued()[0];
    hidden(fixture.server.id(), destination, fixture.caller.id());
    memory::close_user_address_space_handle(fixture.server).unwrap();
    reclaimed(fixture.server.id(), destination);
    assert_eq!(
        poll_reply(fixture.caller.id(), pending).unwrap().unwrap().result,
        REPLY_ENDPOINT_CLOSED
    );
    assert_eq!(memory::budget::used(fixture.caller), memory::budget::Amount::default());
    memory::close_user_address_space_handle(fixture.caller).unwrap();
}

fn queued_vector() {
    for call in [false, true] {
        for deliver in [false, true] {
            let fixture = Fixture::new();
            let mut descriptor = Vec::new();
            descriptor.extend_from_slice(&2u16.to_le_bytes());
            for mode in [0u32, 1] {
                let source = object::allocate(fixture.caller.id(), 1).unwrap();
                object::write_bytes(fixture.caller.id(), source, &[mode as u8]).unwrap();
                descriptor.extend_from_slice(&source.to_le_bytes());
                descriptor.extend_from_slice(&mode.to_le_bytes());
                descriptor.extend_from_slice(&0u32.to_le_bytes());
            }
            let vector = object::allocate(fixture.caller.id(), 1).unwrap();
            object::write_bytes(fixture.caller.id(), vector, &descriptor).unwrap();
            let pending = if call {
                Some(vector_call(fixture.caller.id(), fixture.connection, 1, 0, vector).unwrap())
            } else {
                vector_send(fixture.caller.id(), fixture.connection, 1, 0, vector).unwrap();
                None
            };
            let destinations = fixture.queued();
            for &cap in &destinations {
                hidden(fixture.server.id(), cap, fixture.caller.id());
            }
            if deliver {
                let result = object::allocate(fixture.server.id(), 1).unwrap();
                receive_vec(fixture.server.id(), fixture.endpoint, result).unwrap();
                let bytes = object::snapshot_bytes(fixture.server.id(), result, 18).unwrap();
                assert_eq!(&bytes[..2], &2u16.to_le_bytes());
                for (index, &cap) in destinations.iter().enumerate() {
                    assert_eq!(&bytes[2 + index * 8..10 + index * 8], &cap.to_le_bytes());
                    assert_eq!(
                        object::snapshot_bytes(fixture.server.id(), cap, 1).unwrap(),
                        [index as u8]
                    );
                }
            }
            close_cap(fixture.server.id(), fixture.endpoint).unwrap();
            if let Some(pending) = pending {
                close_cap(fixture.caller.id(), pending).unwrap();
            }
            for cap in destinations {
                if deliver {
                    object::close_cap(fixture.server.id(), cap).unwrap();
                }
                reclaimed(fixture.server.id(), cap);
            }
            fixture.close();
        }
    }
}

fn returned_memory() {
    for observed in [false, true] {
        let fixture = Fixture::new();
        let pending = scalar_call(fixture.caller.id(), fixture.connection, 1, 0).unwrap();
        let token = receive(fixture.server.id(), fixture.endpoint).unwrap().reply.unwrap();
        let source = object::allocate(fixture.server.id(), 1).unwrap();
        object::write_bytes(fixture.server.id(), source, &[0x5a]).unwrap();
        reply_with_memory_move(fixture.server.id(), token, source, 42).unwrap();
        let destination = {
            let ipc = IPC.read();
            let Capability::PendingCall {
                call,
            } = ipc.cap(fixture.caller.id(), pending).unwrap()
            else {
                unreachable!()
            };
            ipc.pending_calls[&call].result.unwrap().memory.unwrap()
        };
        hidden(fixture.caller.id(), destination, fixture.server.id());
        // Readiness waiting alone does not observe or transfer the result.
        wait_reply(fixture.caller.id(), pending).unwrap();
        hidden(fixture.caller.id(), destination, fixture.server.id());
        if observed {
            assert_eq!(
                poll_reply(fixture.caller.id(), pending).unwrap().unwrap().memory,
                Some(destination)
            );
            object::map_any(fixture.caller.id(), destination, true).unwrap();
        }
        close_cap(fixture.caller.id(), pending).unwrap();
        if observed {
            assert_eq!(
                object::snapshot_bytes(fixture.caller.id(), destination, 1).unwrap(),
                [0x5a]
            );
            object::unmap(fixture.caller.id(), destination).unwrap();
            object::close_cap(fixture.caller.id(), destination).unwrap();
        }
        reclaimed(fixture.caller.id(), destination);
        assert_eq!(memory::budget::used(fixture.server), memory::budget::Amount::default());
        fixture.close();
    }
}
