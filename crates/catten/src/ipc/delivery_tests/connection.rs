//! Connection delivery, indirect cleanup and source-mint regression fixtures.
use super::*;
use crate::capability::admission_tests::test_namespace_used;

fn hidden(asid: AddressSpaceId, cap: CapabilityId, peer: AddressSpaceId) {
    assert!(crate::capability::contains(asid, cap, crate::capability::ObjectKind::Ipc));
    let before = (test_namespace_used(asid), test_namespace_used(peer));
    let records = IPC.read().caps[&asid].record_budget.used();
    assert_eq!(IPC.read().cap(asid, cap), Err(IpcError::UnknownCapability));
    assert_eq!(scalar_send(asid, cap, 2, 0), Err(IpcError::UnknownCapability));
    assert_eq!(scalar_call(asid, cap, 2, 0), Err(IpcError::UnknownCapability));
    assert_eq!(connection_mint(asid, cap, ConnectionRights::ALL), Err(IpcError::UnknownCapability));
    assert_eq!(
        connection_delegate(asid, cap, peer, ConnectionRights::ALL),
        Err(IpcError::UnknownCapability)
    );
    assert_eq!(connection_endpoint_owner(asid, cap), Err(IpcError::UnknownCapability));
    assert_eq!(watch_connection_closed(asid, cap), Err(IpcError::UnknownCapability));
    assert_eq!(connection_close_watch_count(asid, cap), Err(IpcError::UnknownCapability));
    assert_eq!(close_cap(asid, cap), Err(IpcError::UnknownCapability));
    assert_eq!((test_namespace_used(asid), test_namespace_used(peer)), before);
    assert_eq!(IPC.read().caps[&asid].record_budget.used(), records);
    assert!(crate::capability::contains(asid, cap, crate::capability::ObjectKind::Ipc));
}

fn reclaimed(asid: AddressSpaceId, cap: CapabilityId) {
    assert!(!crate::capability::contains(asid, cap, crate::capability::ObjectKind::Ipc));
}

fn queued(fixture: &Fixture) -> CapabilityId {
    let ipc = IPC.read();
    let Capability::Endpoint {
        endpoint,
        ..
    } = ipc.cap(fixture.server.id(), fixture.endpoint).unwrap()
    else {
        unreachable!()
    };
    ipc.endpoints[&endpoint].queue.front().unwrap().connection.unwrap()
}

pub(super) fn run() {
    queued_connection();
    namespace_cleanup();
    returned_connection();
    crate::logln!(
        "[IPC connection delivery] guessed send/call/mint/watch/close rejected without charge \
         changes; receive rollback, queue/result cleanup, repeated observation and delivered \
         attenuation passed"
    );
}

fn queued_connection() {
    for copy in [false, true] {
        for delivered in [false, true] {
            for endpoint_close in [false, true] {
                let fixture = Fixture::new();
                let target = endpoint_create(fixture.caller.id(), 2, 1, 4).unwrap();
                let source = copy.then(|| object::allocate(fixture.caller.id(), 1).unwrap());
                let call = if let Some(source) = source {
                    object::write_bytes(fixture.caller.id(), source, &[0xa7]).unwrap();
                    scalar_call_with_connection_copy(
                        fixture.caller.id(),
                        fixture.connection,
                        1,
                        0,
                        target,
                        ConnectionRights::ALL,
                        source,
                    )
                    .unwrap()
                } else {
                    scalar_call_with_connection(
                        fixture.caller.id(),
                        fixture.connection,
                        1,
                        0,
                        target,
                        ConnectionRights::ALL,
                    )
                    .unwrap()
                };
                let grant = queued(&fixture);
                let sponsor = IPC.read().caps[&fixture.caller.id()].record_budget.clone();
                hidden(fixture.server.id(), grant, fixture.caller.id());
                assert_eq!(endpoint_status(fixture.caller.id(), target).unwrap().1, 0);
                let invalid_result = if copy {
                    fixture.queued()[0]
                } else {
                    object::allocate(fixture.caller.id(), 1).unwrap()
                };
                assert_eq!(
                    receive_vec(fixture.server.id(), fixture.endpoint, invalid_result),
                    Err(IpcError::MemoryTransferFailed)
                );
                assert_eq!(queued(&fixture), grant);
                hidden(fixture.server.id(), grant, fixture.caller.id());
                let child = if delivered {
                    let message = receive(fixture.server.id(), fixture.endpoint).unwrap();
                    assert_eq!(message.connection, Some(grant));
                    if let Some(memory) = message.memory {
                        assert_eq!(
                            object::snapshot_bytes(fixture.server.id(), memory, 1).unwrap(),
                            [0xa7]
                        );
                        object::close_cap(fixture.server.id(), memory).unwrap();
                    }
                    Some(
                        connection_delegate(
                            fixture.server.id(),
                            grant,
                            fixture.caller.id(),
                            ConnectionRights::SEND,
                        )
                        .unwrap(),
                    )
                } else {
                    None
                };
                if endpoint_close {
                    close_cap(fixture.server.id(), fixture.endpoint).unwrap();
                }
                close_cap(fixture.caller.id(), call).unwrap();
                if let Some(child) = child {
                    // Once delivered, minting is legitimate and its child survives.
                    scalar_send(fixture.caller.id(), child, 3, 7).unwrap();
                    assert_eq!(receive(fixture.caller.id(), target).unwrap().arg0, 7);
                    assert_eq!(
                        scalar_call(fixture.caller.id(), child, 3, 0),
                        Err(IpcError::PermissionDenied)
                    );
                    close_cap(fixture.server.id(), grant).unwrap();
                    close_cap(fixture.caller.id(), child).unwrap();
                }
                reclaimed(fixture.server.id(), grant);
                // Call/grant sponsorship has returned to its original owner.
                assert_eq!(sponsor.used(), [0, 0, 0]);
                fixture.close();
            }
        }
    }
}

fn namespace_cleanup() {
    for server_close in [false, true] {
        let fixture = Fixture::new();
        let call = scalar_call_with_connection(
            fixture.caller.id(),
            fixture.connection,
            1,
            0,
            fixture.endpoint,
            ConnectionRights::SEND | ConnectionRights::MINT_CONNECTION,
        )
        .unwrap();
        let grant = queued(&fixture);
        hidden(fixture.server.id(), grant, fixture.caller.id());
        let sponsor = IPC.read().caps[&fixture.caller.id()].record_budget.clone();
        if server_close {
            memory::close_user_address_space_handle(fixture.server).unwrap();
            reclaimed(fixture.server.id(), grant);
            assert_eq!(
                poll_reply(fixture.caller.id(), call).unwrap().unwrap().result,
                REPLY_ENDPOINT_CLOSED
            );
            close_cap(fixture.caller.id(), call).unwrap();
            memory::close_user_address_space_handle(fixture.caller).unwrap();
        } else {
            memory::close_user_address_space_handle(fixture.caller).unwrap();
            reclaimed(fixture.server.id(), grant);
            assert_eq!(receive(fixture.server.id(), fixture.endpoint), Err(IpcError::NoMessage));
            memory::close_user_address_space_handle(fixture.server).unwrap();
        }
        assert_eq!(sponsor.used(), [0, 0, 0]);
    }
}

fn returned_connection() {
    for observed in [false, true] {
        for root_close in [false, true] {
            let fixture = Fixture::new();
            let target = endpoint_create(fixture.server.id(), 3, 1, 4).unwrap();
            let call = scalar_call(fixture.caller.id(), fixture.connection, 1, 0).unwrap();
            let token = receive(fixture.server.id(), fixture.endpoint).unwrap().reply.unwrap();
            reply_with_connection(fixture.server.id(), token, target, ConnectionRights::ALL, 42)
                .unwrap();
            let grant = {
                let ipc = IPC.read();
                let Capability::PendingCall {
                    call,
                } = ipc.cap(fixture.caller.id(), call).unwrap()
                else {
                    unreachable!()
                };
                ipc.pending_calls[&call].result.unwrap().cap.unwrap()
            };
            let sponsor = IPC.read().caps[&fixture.caller.id()].record_budget.clone();
            hidden(fixture.caller.id(), grant, fixture.server.id());
            wait_reply(fixture.caller.id(), call).unwrap();
            hidden(fixture.caller.id(), grant, fixture.server.id());
            if observed {
                let result = poll_reply(fixture.caller.id(), call).unwrap().unwrap();
                assert_eq!(result.cap, Some(grant));
                assert_eq!(poll_reply(fixture.caller.id(), call), Ok(Some(result)));
                scalar_send(fixture.caller.id(), grant, 5, 0).unwrap();
                receive(fixture.server.id(), target).unwrap();
                // Repeat polling must not republish an already consumed grant.
                close_cap(fixture.caller.id(), grant).unwrap();
                assert_eq!(poll_reply(fixture.caller.id(), call), Ok(Some(result)));
            }
            if root_close {
                memory::close_user_address_space_handle(fixture.caller).unwrap();
                reclaimed(fixture.caller.id(), grant);
                memory::close_user_address_space_handle(fixture.server).unwrap();
            } else {
                close_cap(fixture.caller.id(), call).unwrap();
                reclaimed(fixture.caller.id(), grant);
                assert_eq!(sponsor.used(), [0, 0, 0]);
                fixture.close();
            }
            assert_eq!(sponsor.used(), [0, 0, 0]);
        }
    }
}
