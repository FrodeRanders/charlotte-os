//! Returned-memory escrow, close and publication regression fixtures.
use super::*;
use crate::capability::admission_tests::{
    test_fill_remaining_namespace as fill,
    test_free_fixture_slot as free,
    test_namespace_used as used,
};

pub(super) fn run() {
    close_and_return();
    publication_rollback();
    preparation_failure();
    undelivered_source();
    borrowed_source();
    staged_close_and_joint_publication();
    source_fence();
    crate::logln!(
        "[IPC returned memory] hidden escrow/backing, unlocked source close, observed/unobserved \
         cleanup, quota/loan/publication rollback, delivered-source checks, staged close, joint \
         authority and source-fence release passed"
    );
}

fn source(fixture: &Fixture) -> MemoryObjectCap {
    let cap = object::allocate(fixture.server.id(), 1).unwrap();
    object::write_bytes(fixture.server.id(), cap, &[0xa5; 32]).unwrap();
    cap
}

fn close_and_return() {
    for observed in [false, true] {
        let fixture = Fixture::new(2);
        let memory = source(&fixture);
        let operation =
            PreparedReply::prepare_with_memory(fixture.server.id(), fixture.reply, memory).unwrap();
        let destination = operation.memory.as_ref().unwrap().target_cap();
        assert_eq!(
            object::info(fixture.server.id(), memory),
            Err(MemoryObjectError::UnknownCapability)
        );
        assert_eq!(
            object::info(fixture.caller.id(), destination),
            Err(MemoryObjectError::UnknownCapability)
        );
        // Internal serialized cleanup rejects without consuming the escrow.
        assert_eq!(
            object::try_close_cap(fixture.server.id(), memory),
            Err(MemoryObjectError::LendingActive)
        );
        let mut operation = Some(operation);
        let mut waits = 0;
        assert_eq!(
            object::close_cap_with_wait(fixture.server.id(), memory, || {
                unlocked();
                assert_eq!(
                    memory::close_user_address_space_handle(fixture.server),
                    Err(AddressSpaceCloseError::OperationsInFlight)
                );
                assert_eq!(
                    memory::close_user_address_space_handle(fixture.caller),
                    Err(AddressSpaceCloseError::OperationsInFlight)
                );
                operation
                    .take()
                    .unwrap()
                    .finish_with(42, |loan| loan.finish_observed(unlocked))
                    .unwrap();
                waits += 1;
            }),
            Err(MemoryObjectError::UnknownCapability)
        );
        assert_eq!(waits, 1);
        assert_eq!(
            object::info(fixture.caller.id(), destination),
            Err(MemoryObjectError::UnknownCapability)
        );
        assert_eq!(
            object::snapshot_bytes(fixture.caller.id(), destination, 32),
            Err(MemoryObjectError::UnknownCapability)
        );
        if observed {
            assert_eq!(
                poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap().memory,
                Some(destination)
            );
        }
        close_cap(fixture.caller.id(), fixture.call).unwrap();
        if observed {
            assert_eq!(
                object::snapshot_bytes(fixture.caller.id(), destination, 32).unwrap(),
                [0xa5; 32]
            );
            object::close_cap(fixture.caller.id(), destination).unwrap();
        } else {
            assert_eq!(
                object::info(fixture.caller.id(), destination),
                Err(MemoryObjectError::UnknownCapability)
            );
        }
        assert_eq!(memory::budget::used(fixture.server), memory::budget::Amount::default());
        fixture.close();
    }
}

fn publication_rollback() {
    let fixture = Fixture::new(2);
    let memory = source(&fixture);
    let before = used(fixture.caller.id());
    let mut operation = Some(
        PreparedReply::prepare_with_memory(fixture.server.id(), fixture.reply, memory).unwrap(),
    );
    let destination = operation.as_ref().unwrap().memory.as_ref().unwrap().target_cap();
    let mut waits = 0;
    object::close_cap_with_wait(fixture.server.id(), memory, || {
        unlocked();
        assert_eq!(
            operation.take().unwrap().finish_with_publication(
                1,
                |loan| loan.finish_observed(unlocked),
                |_, _| Err(IpcError::MemoryTransferFailed)
            ),
            Err(IpcError::MemoryTransferFailed)
        );
        // Source authority and retention pin are restored/completed together.
        // Close must not wake to a still-pinned, ambiguously usable capability.
        assert_eq!(object::snapshot_bytes(fixture.server.id(), memory, 32).unwrap(), [0xa5; 32]);
        waits += 1;
    })
    .unwrap();
    assert_eq!(waits, 1);
    assert_eq!(used(fixture.caller.id()), before);
    assert_eq!(
        object::info(fixture.caller.id(), destination),
        Err(MemoryObjectError::UnknownCapability)
    );
    assert_eq!(poll_reply(fixture.caller.id(), fixture.call), Ok(None));
    reply(fixture.server.id(), fixture.reply, 42).unwrap();
    fixture.close();
}

fn preparation_failure() {
    let fixture = Fixture::new(2);
    let memory = source(&fixture);
    fill(fixture.caller.id());
    assert_eq!(
        reply_with_memory_move(fixture.server.id(), fixture.reply, memory, 1),
        Err(IpcError::MemoryTransferFailed)
    );
    assert_eq!(object::snapshot_bytes(fixture.server.id(), memory, 32).unwrap(), [0xa5; 32]);
    assert!(object::info(fixture.server.id(), fixture.borrows[0].borrower_cap).unwrap().mapped);
    free(fixture.caller.id());
    let before = used(fixture.caller.id());
    let borrow = fixture.borrows[1];
    let blocker = LoanRevocation::prepare(
        borrow.owner,
        borrow.owner_cap,
        borrow.borrower,
        borrow.borrower_cap,
    )
    .unwrap();
    assert_eq!(
        reply_with_memory_move(fixture.server.id(), fixture.reply, memory, 1),
        Err(IpcError::MemoryTransferFailed)
    );
    assert_eq!(used(fixture.caller.id()), before);
    object::write_bytes(fixture.server.id(), memory, &[0x5a; 32]).unwrap();
    blocker.cancel_prepared();
    // The borrowed input is not owned return authority; reject without mutation.
    assert_eq!(
        reply_with_memory_move(
            fixture.server.id(),
            fixture.reply,
            fixture.borrows[0].borrower_cap,
            1
        ),
        Err(IpcError::MemoryTransferFailed)
    );
    reply_with_memory_move(fixture.server.id(), fixture.reply, memory, 1).unwrap();
    let returned = poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap().memory.unwrap();
    assert_eq!(object::snapshot_bytes(fixture.caller.id(), returned, 32).unwrap(), [0x5a; 32]);
    fixture.close();
}

fn undelivered_source() {
    for queued in [false, true] {
        let fixture = Fixture::new(1);
        let endpoint = endpoint_create(fixture.server.id(), 20, 1, 4).unwrap();
        let connection =
            connection_mint(fixture.server.id(), endpoint, ConnectionRights::ALL).unwrap();
        let memory = source(&fixture);
        let prior = if queued {
            scalar_call_with_memory_move(fixture.server.id(), connection, 1, 0, memory).unwrap()
        } else {
            let prior = scalar_call(fixture.server.id(), connection, 1, 0).unwrap();
            let token = receive(fixture.server.id(), endpoint).unwrap().reply.unwrap();
            reply_with_memory_move(fixture.server.id(), token, memory, 1).unwrap();
            prior
        };
        let memory = {
            let ipc = IPC.read();
            if queued {
                let Capability::Endpoint {
                    endpoint,
                    ..
                } = ipc.cap(fixture.server.id(), endpoint).unwrap()
                else {
                    unreachable!();
                };
                ipc.endpoints[&endpoint].queue.front().unwrap().memory[0]
            } else {
                let Capability::PendingCall {
                    call,
                } = ipc.cap(fixture.server.id(), prior).unwrap()
                else {
                    unreachable!();
                };
                ipc.pending_calls[&call].result.unwrap().memory.unwrap()
            }
        };
        let before = used(fixture.caller.id());
        assert!(matches!(
            PreparedReply::prepare_with_memory(fixture.server.id(), fixture.reply, memory),
            Err(IpcError::MemoryTransferFailed)
        ));
        assert_eq!(used(fixture.caller.id()), before);
        assert_eq!(
            object::info(fixture.server.id(), memory),
            Err(MemoryObjectError::UnknownCapability)
        );
        if queued {
            assert_eq!(receive(fixture.server.id(), endpoint).unwrap().memory, Some(memory));
        } else {
            assert_eq!(
                poll_reply(fixture.server.id(), prior).unwrap().unwrap().memory,
                Some(memory)
            );
        }
        let operation =
            PreparedReply::prepare_with_memory(fixture.server.id(), fixture.reply, memory).unwrap();
        close_cap(fixture.server.id(), prior).unwrap();
        operation.finish(1).unwrap();
        fixture.close();
    }
}

fn staged_close_and_joint_publication() {
    let fixture = Fixture::new(1);
    let memory = source(&fixture);
    let source_endpoint = endpoint_create(fixture.server.id(), 21, 1, 4).unwrap();
    let operation = PreparedReply::prepare_with_outputs(
        fixture.server.id(),
        fixture.reply,
        Some((source_endpoint, ConnectionRights::SEND)),
        Some(memory),
    )
    .unwrap();
    let connection = operation.connection.as_ref().unwrap().grant.authority.identity();
    let destination = operation.memory.as_ref().unwrap().target_cap();
    let closing = ClosingAddressSpace::begin(fixture.caller).unwrap();
    let CloseProgress::Pending(closing) = closing.poll().unwrap() else {
        panic!("reply did not retain closing root");
    };
    operation.finish_with(42, |loan| loan.finish_observed(unlocked)).unwrap();
    let result = poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap();
    assert_eq!((result.cap, result.memory), (Some(connection), Some(destination)));
    assert!(matches!(closing.poll().unwrap(), CloseProgress::Complete));
    memory::close_user_address_space_handle(fixture.server).unwrap();
}

fn borrowed_source() {
    let fixture = Fixture::new(2);
    let borrowed = fixture.borrows[0].borrower_cap;
    let before = used(fixture.caller.id());
    // Visible borrowed bytes do not confer ownership of returnable backing.
    assert!(matches!(
        object::prepare_copy(fixture.server.id(), borrowed, fixture.caller.id()),
        Err(MemoryObjectError::WrongOwner)
    ));
    assert!(matches!(
        PreparedReply::prepare_with_memory(fixture.server.id(), fixture.reply, borrowed),
        Err(IpcError::MemoryTransferFailed)
    ));
    assert_eq!(used(fixture.caller.id()), before);
    assert_eq!(poll_reply(fixture.caller.id(), fixture.call), Ok(None));
    for borrow in &fixture.borrows {
        assert!(object::info(borrow.borrower, borrow.borrower_cap).unwrap().mapped);
        assert!(object::info(borrow.owner, borrow.owner_cap).unwrap().lent);
    }
    // Ordinary reply can still revoke both loans after failed qualification.
    reply(fixture.server.id(), fixture.reply, 7).unwrap();
    let result = poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap();
    assert_eq!((result.result, result.memory), (7, None));
    fixture.close();
}

fn source_fence() {
    let fixture = Fixture::new(1);
    let third = crate::service::loader::create_user_address_space_handle();
    let memory = source(&fixture);
    let mut transfer =
        object::prepare_loan(fixture.server.id(), memory, fixture.caller.id(), false).unwrap();
    object::commit_transfers(core::slice::from_mut(&mut transfer)).unwrap();
    let loan = transfer.target_cap();
    // A committed read loan restores source authority before the transaction
    // owner releases its pin. A second preparation must not steal that fence.
    assert!(matches!(
        object::prepare_loan(fixture.server.id(), memory, third.id(), false),
        Err(MemoryObjectError::LendingActive)
    ));
    assert_eq!(
        object::try_close_cap(fixture.server.id(), memory),
        Err(MemoryObjectError::LendingActive)
    );
    let mut transfer = Some(transfer);
    let mut waits = 0;
    assert_eq!(
        object::close_cap_with_wait(fixture.server.id(), memory, || {
            unlocked();
            drop(transfer.take().unwrap());
            waits += 1;
        }),
        Err(MemoryObjectError::LendingActive)
    );
    assert_eq!(waits, 1);
    let other = object::lend_read(fixture.server.id(), memory, third.id()).unwrap();
    object::revoke_lend(fixture.server.id(), memory, fixture.caller.id(), loan).unwrap();
    object::revoke_lend(fixture.server.id(), memory, third.id(), other).unwrap();
    object::close_cap(fixture.server.id(), memory).unwrap();
    memory::close_user_address_space_handle(third).unwrap();
    fixture.close();
}
