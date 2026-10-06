//! Source-close and destination-publication fixtures using real namespaces.
use super::*;
use crate::capability::admission_tests::{
    test_fill_remaining_namespace as fill,
    test_free_fixture_slot as free,
    test_namespace_used as used,
};

pub(super) fn run() {
    source_close_wait();
    external_owner_close();
    unobserved_return_cleanup();
    preparation_failure();
    undelivered_source();
    publication_failure();
    crate::logln!(
        "[IPC returned connection] source endpoint/connection close waits, external owner close, \
         delivered-source checks, hidden attenuated grants, observed/unobserved cleanup, \
         quota/preparation rollback and rejected publication passed"
    );
}

fn source_close_wait() {
    for delegated in [false, true] {
        let fixture = Fixture::new(2);
        let source_endpoint = endpoint_create(fixture.server.id(), 4, 1, 4).unwrap();
        let requested = ConnectionRights::ALL;
        let granted = if delegated {
            ConnectionRights::SEND | ConnectionRights::MINT_CONNECTION
        } else {
            requested
        };
        let source = if delegated {
            connection_mint(fixture.server.id(), source_endpoint, granted).unwrap()
        } else {
            source_endpoint
        };
        let before = used(fixture.caller.id());
        let operation = PreparedReply::prepare_with_connection(
            fixture.server.id(),
            fixture.reply,
            Some((source, requested)),
        )
        .unwrap();
        let destination = operation.connection.as_ref().unwrap().grant.authority.identity();
        assert_eq!(used(fixture.caller.id()), before + 1);
        assert!(matches!(
            IPC.read().cap(fixture.caller.id(), destination),
            Err(IpcError::UnknownCapability)
        ));
        let mut operation = Some(operation);
        let mut waits = 0;
        close_cap_with_wait(fixture.server.id(), source, || {
            unlocked();
            assert!(IPC.read().cap(fixture.server.id(), source).is_ok());
            operation
                .take()
                .unwrap()
                .finish_with(42, |loan| loan.finish_observed(unlocked))
                .unwrap();
            waits += 1;
        })
        .unwrap();
        assert_eq!(waits, 1);
        let result = poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap();
        assert_eq!(result.result, 42);
        assert_eq!(result.cap, Some(destination));
        let Capability::Connection {
            rights,
            ..
        } = IPC.read().cap(fixture.caller.id(), destination).unwrap()
        else {
            panic!("returned connection missing");
        };
        assert_eq!(rights, granted);
        assert!(matches!(
            IPC.read().cap(fixture.server.id(), source),
            Err(IpcError::UnknownCapability)
        ));
        if delegated {
            scalar_send(fixture.caller.id(), destination, 9, 0).unwrap();
            assert_eq!(receive(fixture.server.id(), source_endpoint).unwrap().opcode, 9);
            assert_eq!(
                scalar_call(fixture.caller.id(), destination, 9, 0),
                Err(IpcError::PermissionDenied)
            );
        } else {
            // Close waited until publication. The delegated connection now
            // names a closed but still retained endpoint, not a dangling ID.
            assert_eq!(
                scalar_send(fixture.caller.id(), destination, 9, 0),
                Err(IpcError::EndpointClosed)
            );
        }
        close_cap(fixture.caller.id(), fixture.call).unwrap();
        assert!(
            IPC.read().cap(fixture.caller.id(), destination).is_ok(),
            "observed result was reclaimed"
        );
        close_cap(fixture.caller.id(), destination).unwrap();
        fixture.close();
    }
}

fn unobserved_return_cleanup() {
    let fixture = Fixture::new(1);
    let source = endpoint_create(fixture.server.id(), 5, 1, 4).unwrap();
    let mut operation = Some(
        PreparedReply::prepare_with_connection(
            fixture.server.id(),
            fixture.reply,
            Some((source, ConnectionRights::ALL)),
        )
        .unwrap(),
    );
    let destination =
        operation.as_ref().unwrap().connection.as_ref().unwrap().grant.authority.identity();
    let sponsor = IPC.read().caps[&fixture.caller.id()].record_budget.clone();
    close_cap_with_wait(fixture.caller.id(), fixture.call, || {
        unlocked();
        operation.take().unwrap().finish(1).unwrap();
    })
    .unwrap();
    assert!(matches!(
        IPC.read().cap(fixture.caller.id(), destination),
        Err(IpcError::UnknownCapability)
    ));
    assert_eq!(sponsor.used(), [0, 0, 0]);
    assert!(IPC.read().cap(fixture.server.id(), source).is_ok());
    fixture.close();
}

fn external_owner_close() {
    let fixture = Fixture::new(1);
    let owner = crate::service::loader::create_user_address_space_handle();
    let endpoint = endpoint_create(owner.id(), 9, 1, 4).unwrap();
    let source =
        connection_delegate(owner.id(), endpoint, fixture.server.id(), ConnectionRights::ALL)
            .unwrap();
    let operation = PreparedReply::prepare_with_connection(
        fixture.server.id(),
        fixture.reply,
        Some((source, ConnectionRights::SEND)),
    )
    .unwrap();
    operation
        .finish_with(1, |loan| {
            unlocked();
            // A delegated source is sufficient to retain its endpoint even when
            // the endpoint's owning domain closes during the unlocked interval.
            memory::close_user_address_space_handle(owner).unwrap();
            loan.finish_observed(unlocked)
        })
        .unwrap();
    let result = poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap().cap.unwrap();
    assert_eq!(scalar_send(fixture.caller.id(), result, 1, 0), Err(IpcError::EndpointClosed));
    fixture.close();
}

fn preparation_failure() {
    let fixture = Fixture::new(2);
    let source = endpoint_create(fixture.server.id(), 6, 1, 4).unwrap();
    let sponsor = IPC.read().caps[&fixture.caller.id()].record_budget.clone();
    let before = sponsor.used();
    fill(fixture.caller.id());
    assert_eq!(
        reply_with_connection(fixture.server.id(), fixture.reply, source, ConnectionRights::ALL, 1),
        Err(IpcError::ResourceLimit)
    );
    assert_eq!(sponsor.used(), before);
    assert_eq!(poll_reply(fixture.caller.id(), fixture.call), Ok(None));
    assert!(object::info(fixture.server.id(), fixture.borrows[0].borrower_cap).unwrap().mapped);
    free(fixture.caller.id());
    let namespace_before = used(fixture.caller.id());
    let borrow = fixture.borrows[1];
    let blocker = LoanRevocation::prepare(
        borrow.owner,
        borrow.owner_cap,
        borrow.borrower,
        borrow.borrower_cap,
    )
    .unwrap();
    assert_eq!(
        reply_with_connection(fixture.server.id(), fixture.reply, source, ConnectionRights::ALL, 1),
        Err(IpcError::MemoryTransferFailed)
    );
    assert_eq!(sponsor.used(), before);
    assert_eq!(used(fixture.caller.id()), namespace_before);
    assert!(IPC.read().cap(fixture.server.id(), source).is_ok());
    blocker.cancel_prepared();
    reply_with_connection(fixture.server.id(), fixture.reply, source, ConnectionRights::ALL, 1)
        .unwrap();
    assert!(poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap().cap.is_some());
    fixture.close();
}

fn publication_failure() {
    let fixture = Fixture::new(2);
    let source = endpoint_create(fixture.server.id(), 7, 1, 4).unwrap();
    let before = used(fixture.caller.id());
    let sponsor = IPC.read().caps[&fixture.caller.id()].record_budget.clone();
    let records = sponsor.used();
    let operation = PreparedReply::prepare_with_connection(
        fixture.server.id(),
        fixture.reply,
        Some((source, ConnectionRights::ALL)),
    )
    .unwrap();
    let destination = operation.connection.as_ref().unwrap().grant.authority.identity();
    assert_eq!(
        operation.finish_with_publication(
            1,
            |loan| loan.finish_observed(unlocked),
            |_, _| Err(IpcError::MemoryTransferFailed)
        ),
        Err(IpcError::MemoryTransferFailed)
    );
    assert_eq!(poll_reply(fixture.caller.id(), fixture.call), Ok(None));
    assert_eq!(sponsor.used(), records);
    assert_eq!(used(fixture.caller.id()), before);
    assert!(matches!(
        IPC.read().cap(fixture.caller.id(), destination),
        Err(IpcError::UnknownCapability)
    ));
    for borrow in &fixture.borrows {
        assert_eq!(
            object::info(fixture.server.id(), borrow.borrower_cap),
            Err(MemoryObjectError::UnknownCapability)
        );
    }
    close_cap(fixture.server.id(), source).unwrap();
    reply(fixture.server.id(), fixture.reply, 42).unwrap();
    assert_eq!(poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap().result, 42);
    fixture.close();
}

fn undelivered_source() {
    for queued in [false, true] {
        let fixture = Fixture::new(1);
        let endpoint = endpoint_create(fixture.server.id(), 10, 1, 4).unwrap();
        let connection =
            connection_mint(fixture.server.id(), endpoint, ConnectionRights::ALL).unwrap();
        let prior = if queued {
            scalar_call_with_connection(
                fixture.server.id(),
                connection,
                1,
                0,
                endpoint,
                ConnectionRights::ALL,
            )
            .unwrap()
        } else {
            let prior = scalar_call(fixture.server.id(), connection, 1, 0).unwrap();
            let reply = receive(fixture.server.id(), endpoint).unwrap().reply.unwrap();
            reply_with_connection(fixture.server.id(), reply, endpoint, ConnectionRights::ALL, 1)
                .unwrap();
            prior
        };
        let source = {
            let ipc = IPC.read();
            if queued {
                let Capability::Endpoint {
                    endpoint,
                    ..
                } = ipc.cap(fixture.server.id(), endpoint).unwrap()
                else {
                    unreachable!();
                };
                ipc.endpoints[&endpoint].queue.front().unwrap().connection.unwrap()
            } else {
                let Capability::PendingCall {
                    call,
                } = ipc.cap(fixture.server.id(), prior).unwrap()
                else {
                    unreachable!();
                };
                ipc.pending_calls[&call].result.unwrap().cap.unwrap()
            }
        };
        let before = used(fixture.caller.id());
        assert!(matches!(
            PreparedReply::prepare_with_connection(
                fixture.server.id(),
                fixture.reply,
                Some((source, ConnectionRights::ALL))
            ),
            Err(IpcError::UnknownCapability)
        ));
        assert_eq!(used(fixture.caller.id()), before);
        assert!(object::info(fixture.server.id(), fixture.borrows[0].borrower_cap).unwrap().mapped);
        if queued {
            let message = receive(fixture.server.id(), endpoint).unwrap();
            assert_eq!(message.connection, Some(source));
            // Closing the earlier call can now revoke its reply token but
            // cannot reclaim the connection already delivered to the receiver.
        } else {
            assert_eq!(poll_reply(fixture.server.id(), prior).unwrap().unwrap().cap, Some(source));
        }
        let operation = PreparedReply::prepare_with_connection(
            fixture.server.id(),
            fixture.reply,
            Some((source, ConnectionRights::ALL)),
        )
        .unwrap();
        close_cap(fixture.server.id(), prior).unwrap();
        operation.finish(1).unwrap();
        assert!(poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap().cap.is_some());
        fixture.close();
    }
}
