//! Kernel ABI fixtures for retirement storage and unlocked physical release.
use super::*;

fn unlocked(owner: object::RetiredMemory) {
    assert!(IPC.try_write().is_some(), "backing release held IPC");
    object::retirement_tests::assert_backing_release_unlocked();
    owner.release().unwrap();
}

pub(super) fn run() {
    submission_failure();
    bulk_vectors();
    call_and_result();
    crate::logln!(
        "[IPC backing retirement] preparation rollback, maximum vectors, bulk queue, call/result \
         release outside IPC passed"
    );
}

fn submission_failure() {
    let fixture = Fixture::new();
    let sources = [
        object::allocate(fixture.caller.id(), 1).unwrap(),
        object::allocate(fixture.caller.id(), 1).unwrap(),
    ];
    let before = memory::budget::used(fixture.caller);
    let records = record_budget::node_used();
    let caps = [fixture.caller, fixture.server]
        .map(|handle| crate::capability::admission_tests::test_namespace_used(handle.id()));
    let mut ipc = IPC.write();
    let Capability::Connection {
        endpoint,
        ..
    } = ipc.cap(fixture.caller.id(), fixture.connection).unwrap()
    else {
        unreachable!()
    };
    let mut call = ipc.stage_call(fixture.caller.id()).unwrap();
    call.connection = Some(
        PreparedConnection::new(
            &mut ipc,
            fixture.caller.id(),
            fixture.server.id(),
            endpoint,
            ConnectionRights::SEND,
        )
        .unwrap(),
    );
    call.attach(
        object::prepare_move(fixture.caller.id(), sources[0], fixture.server.id()).unwrap(),
    )
    .unwrap();
    call.attach(
        object::prepare_copy(fixture.caller.id(), sources[1], fixture.server.id()).unwrap(),
    )
    .unwrap();
    let destinations = call.memory.clone();
    assert!(matches!(
        call.commit_with_memory(&mut ipc, endpoint, 1, 0, |_| Err(IpcError::ResourceLimit)),
        Err(IpcError::ResourceLimit)
    ));
    drop(ipc);
    assert_eq!(memory::budget::used(fixture.caller), before);
    assert_eq!(record_budget::node_used(), records);
    assert_eq!(
        [fixture.caller, fixture.server]
            .map(|handle| crate::capability::admission_tests::test_namespace_used(handle.id())),
        caps
    );
    assert_eq!(endpoint_status(fixture.server.id(), fixture.endpoint).unwrap().1, 0);
    for cap in sources {
        object::write_bytes(fixture.caller.id(), cap, &[0x5a]).unwrap();
    }
    for cap in destinations {
        reclaimed(fixture.server.id(), cap);
    }
    fixture.close();
}

fn bulk_vectors() {
    // Two full vectors exceed a single-vector snapshot. Cleanup reuses each
    // message's admitted metadata rather than allocating or growing its stack.
    let fixture = Fixture::new();
    for _ in 0..2 {
        let mut descriptor = Vec::new();
        descriptor.extend_from_slice(&(CAP_VECTOR_MAX as u16).to_le_bytes());
        for _ in 0..CAP_VECTOR_MAX {
            let source = object::allocate(fixture.caller.id(), 1).unwrap();
            descriptor.extend_from_slice(&source.to_le_bytes());
            descriptor.extend_from_slice(&1u32.to_le_bytes());
            descriptor.extend_from_slice(&0u32.to_le_bytes());
        }
        let vector = object::allocate(fixture.caller.id(), 1).unwrap();
        object::write_bytes(fixture.caller.id(), vector, &descriptor).unwrap();
        vector_send(fixture.caller.id(), fixture.connection, 1, 0, vector).unwrap();
    }
    assert_eq!(memory::budget::used(fixture.caller).objects, 2 * CAP_VECTOR_MAX as u64);
    let mut releases = 0;
    close_cap_with_cleanup(
        fixture.server.id(),
        fixture.endpoint,
        false,
        || panic!("bulk cleanup waited"),
        revoke_memory_borrow,
        |owner| {
            unlocked(owner);
            releases += 1;
        },
    )
    .unwrap();
    assert_eq!(releases, 2 * CAP_VECTOR_MAX);
    assert_eq!(memory::budget::used(fixture.caller), memory::budget::Amount::default());
    fixture.close();
}

fn call_and_result() {
    for returned in [false, true] {
        let fixture = Fixture::new();
        let sponsor;
        let destination;
        let pending;
        if returned {
            pending = scalar_call(fixture.caller.id(), fixture.connection, 1, 0).unwrap();
            let token = receive(fixture.server.id(), fixture.endpoint).unwrap().reply.unwrap();
            let source = object::allocate(fixture.server.id(), 35).unwrap();
            reply_with_memory_move(fixture.server.id(), token, source, 42).unwrap();
            let ipc = IPC.read();
            let Capability::PendingCall {
                call,
            } = ipc.cap(fixture.caller.id(), pending).unwrap()
            else {
                unreachable!()
            };
            destination = ipc.pending_calls[&call].result.unwrap().memory.unwrap();
            sponsor = fixture.server;
        } else {
            let source = object::allocate(fixture.caller.id(), 35).unwrap();
            pending =
                scalar_call_with_memory_move(fixture.caller.id(), fixture.connection, 1, 0, source)
                    .unwrap();
            destination = fixture.queued()[0];
            sponsor = fixture.caller;
        }
        let receiver = if returned {
            fixture.caller
        } else {
            fixture.server
        };
        let mut releases = 0;
        close_cap_with_cleanup(
            fixture.caller.id(),
            pending,
            false,
            || panic!("call cleanup waited"),
            revoke_memory_borrow,
            |owner| {
                reclaimed(receiver.id(), destination);
                assert_eq!(
                    memory::budget::used(sponsor),
                    memory::budget::Amount {
                        pages: 35,
                        objects: 1
                    }
                );
                unlocked(owner);
                releases += 1;
            },
        )
        .unwrap();
        assert_eq!(releases, 1);
        assert_eq!(memory::budget::used(sponsor), memory::budget::Amount::default());
        fixture.close();
    }
}
