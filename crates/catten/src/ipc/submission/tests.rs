//! Raw ABI fixtures: unpublished payload rollback, captured roots and authority.
use super::*;
use crate::memory::{
    self,
    AddressSpaceCloseError,
    PHYSICAL_FRAME_ALLOCATOR,
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
        let endpoint = endpoint_create(server.id(), 0x5354_4147, 1, 4).unwrap();
        let connection =
            connection_delegate(server.id(), endpoint, caller.id(), ConnectionRights::ALL).unwrap();
        Self {
            caller,
            server,
            endpoint,
            connection,
        }
    }

    fn close(self) {
        memory::close_user_address_space_handle(self.caller).unwrap();
        memory::close_user_address_space_handle(self.server).unwrap();
    }
}

fn unlocked(fixture: &Fixture) {
    assert!(IPC.try_write().is_some(), "staged copy rollback held IPC");
    object::retirement_tests::assert_backing_release_unlocked();
    for root in [fixture.caller, fixture.server] {
        assert_eq!(
            memory::close_user_address_space_handle(root),
            Err(AddressSpaceCloseError::OperationsInFlight)
        );
    }
}

pub(crate) fn run() {
    retained_failure();
    partial_vector();
    closed_destination();
    scalar_call_rejection();
    scalar_send_outcomes();
    scalar_send_abandonment();
    crate::logln!(
        "[IPC staged rollback] partial vector, storage/publication rejection, closed destination, \
         unlocked copy release and exact root retention passed"
    );
}

fn retained_failure() {
    for publication in [false, true] {
        let fixture = Fixture::new();
        let source = object::allocate(fixture.caller.id(), 35).unwrap();
        let before = memory::budget::used(fixture.caller);
        let submission =
            Submission::prepare(fixture.caller.id(), fixture.connection, ConnectionRights::CALL)
                .unwrap();
        submission
            .run(|submission| {
                let mut prepared = PreparedCall::try_new(fixture.caller.id()).unwrap();
                prepared.stage(&mut IPC.write(), fixture.caller.id()).unwrap();
                unlocked(&fixture);
                prepared
                    .attach(
                        object::prepare_copy(fixture.caller.id(), source, fixture.server.id())
                            .unwrap(),
                    )
                    .unwrap();
                let destination = prepared.memory[0];
                let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
                if publication {
                    crate::capability::retire_address_space(fixture.server.id());
                }
                let result = {
                    let mut ipc = IPC.write();
                    submission.validate(&ipc).unwrap();
                    prepared.commit_with_memory(
                        &mut ipc,
                        submission.endpoint,
                        1,
                        0,
                        |platform, caps| {
                            if publication {
                                MemoryAttachments::try_new(platform, caps)
                            } else {
                                Err(IpcError::ResourceLimit)
                            }
                        },
                    )
                };
                let error = result.err().expect("fault publication succeeded");
                assert_eq!(
                    error,
                    if publication {
                        IpcError::MemoryTransferFailed
                    } else {
                        IpcError::ResourceLimit
                    }
                );
                unlocked(&fixture);
                assert_eq!(memory::budget::used(fixture.caller).pages, 70);
                prepared.finish();
                assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free + 35);
                assert_eq!(memory::budget::used(fixture.caller), before);
                assert_eq!(
                    object::info(fixture.server.id(), destination),
                    Err(object::MemoryObjectError::UnknownCapability)
                );
                assert_eq!(endpoint_status(fixture.server.id(), fixture.endpoint).unwrap().1, 0);
                object::write_bytes(fixture.caller.id(), source, &[0xa5]).unwrap();
                Ok(())
            })
            .unwrap();
        fixture.close();
    }
}

fn partial_vector() {
    for call in [false, true] {
        let fixture = Fixture::new();
        let source = object::allocate(fixture.caller.id(), 35).unwrap();
        let descriptor = object::allocate(fixture.caller.id(), 1).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2u16.to_le_bytes());
        for cap in [source, u64::MAX] {
            bytes.extend_from_slice(&cap.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
        }
        object::write_bytes(fixture.caller.id(), descriptor, &bytes).unwrap();
        let before = memory::budget::used(fixture.caller);
        let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        assert_eq!(
            vector(fixture.caller.id(), fixture.connection, 1, 0, descriptor, call),
            Err(IpcError::MemoryTransferFailed)
        );
        assert_eq!(memory::budget::used(fixture.caller), before);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
        assert_eq!(endpoint_status(fixture.server.id(), fixture.endpoint).unwrap().1, 0);
        assert!(object::info(fixture.caller.id(), descriptor).is_ok());
        object::write_bytes(fixture.caller.id(), source, &[0x5a]).unwrap();
        fixture.close();
    }
}

fn closed_destination() {
    let fixture = Fixture::new();
    let source = object::allocate(fixture.caller.id(), 35).unwrap();
    let submission =
        Submission::prepare(fixture.caller.id(), fixture.connection, ConnectionRights::SEND)
            .unwrap();
    let before = memory::budget::used(fixture.caller);
    submission
        .run(|submission| {
            let transfer =
                object::prepare_copy(fixture.caller.id(), source, fixture.server.id()).unwrap();
            close_cap(fixture.server.id(), fixture.endpoint).unwrap();
            assert_eq!(submission.validate(&IPC.read()), Err(IpcError::EndpointClosed));
            unlocked(&fixture);
            drop(transfer);
            assert_eq!(memory::budget::used(fixture.caller), before);
            Ok(())
        })
        .unwrap();
    fixture.close();
}

fn prepare_mode(
    source: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
    mode: usize,
) -> Result<object::PreparedTransfer, object::MemoryObjectError> {
    match mode {
        0 => object::prepare_copy(source, cap, target),
        1 => object::prepare_move(source, cap, target),
        2 => object::prepare_loan(source, cap, target, false),
        3 => object::prepare_loan(source, cap, target, true),
        _ => unreachable!(),
    }
}

fn scalar_call_rejection() {
    for mode in 0..4 {
        let fixture = Fixture::new();
        let source = object::allocate(fixture.caller.id(), 35).unwrap();
        let baseline = memory::budget::used(fixture.caller);
        let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let mut destination = None;
        assert_eq!(
            call_memory(
                fixture.caller.id(),
                fixture.connection,
                1,
                0,
                source,
                None,
                |caller, cap, server| {
                    unlocked(&fixture);
                    let transfer = prepare_mode(caller, cap, server, mode)?;
                    destination = Some(transfer.target_cap());
                    close_cap(fixture.server.id(), fixture.endpoint).unwrap();
                    Ok(transfer)
                }
            ),
            Err(IpcError::EndpointClosed)
        );
        assert_eq!(memory::budget::used(fixture.caller), baseline);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
        assert!(object::info(fixture.server.id(), destination.unwrap()).is_err());
        object::write_bytes(fixture.caller.id(), source, &[0xa5]).unwrap();
        fixture.close();
    }
}

fn scalar_send_outcomes() {
    use crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR;
    for mode in 0..2 {
        // Reject attachment storage, close the endpoint, reject authority, or
        // publish successfully. All faults happen after real transfer prep.
        for fault in 0..4 {
            let fixture = Fixture::new();
            let source = object::allocate(fixture.caller.id(), 35).unwrap();
            object::write_bytes(fixture.caller.id(), source, &[0x5a]).unwrap();
            let baseline = memory::budget::used(fixture.caller);
            let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
            let mut send = PreparedSend::new(fixture.caller.id(), fixture.connection).unwrap();
            let preparation = send.prepare(
                source,
                |caller, cap, server| {
                    unlocked(&fixture);
                    prepare_mode(caller, cap, server, mode)
                },
                |platform, cap| {
                    unlocked(&fixture);
                    if fault == 0 {
                        Err(IpcError::ResourceLimit)
                    } else {
                        MemoryAttachments::single(platform, cap)
                    }
                },
            );
            let destination = send.0.transfer.as_ref().unwrap().target_cap();
            let endpoint = send.0.submission.as_ref().unwrap().endpoint;
            if fault == 0 {
                assert_eq!(preparation, Err(IpcError::ResourceLimit));
            } else {
                preparation.unwrap();
                if fault == 1 {
                    close_cap(fixture.server.id(), fixture.endpoint).unwrap();
                }
                if fault == 2 {
                    crate::capability::retire_address_space(fixture.server.id());
                }
                let result = {
                    let _heap = PRIMARY_ALLOCATOR.lock();
                    send.publish(1, 0)
                };
                assert_eq!(
                    result,
                    match fault {
                        1 => Err(IpcError::EndpointClosed),
                        2 => Err(IpcError::MemoryTransferFailed),
                        _ => Ok(()),
                    }
                );
            }
            unlocked(&fixture);
            send.finish().unwrap();
            if fault == 3 {
                let message = receive(fixture.server.id(), fixture.endpoint).unwrap();
                assert_eq!(message.memory, Some(destination));
                assert_eq!(
                    object::snapshot_bytes(fixture.server.id(), destination, 1).unwrap(),
                    [0x5a]
                );
                object::close_cap(fixture.server.id(), destination).unwrap();
                if mode == 0 {
                    object::close_cap(fixture.caller.id(), source).unwrap();
                }
                assert_eq!(memory::budget::used(fixture.caller), memory::budget::Amount::default());
            } else {
                assert_eq!(memory::budget::used(fixture.caller), baseline);
                assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
                assert!(object::info(fixture.server.id(), destination).is_err());
                object::write_bytes(fixture.caller.id(), source, &[0xa5]).unwrap();
                assert_eq!(
                    IPC.read().endpoints.get(&endpoint).map_or(0, |entry| entry.queue.len()),
                    0
                );
            }
            fixture.close();
        }
    }
}

fn scalar_send_abandonment() {
    use crate::memory::{
        ADDRESS_SPACE_LIFECYCLE,
        ADDRESS_SPACE_TABLE,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    };
    for guarded in [false, true] {
        let fixture = Fixture::new();
        let source = object::allocate(fixture.caller.id(), 35).unwrap();
        let mut send = PreparedSend::new(fixture.caller.id(), fixture.connection).unwrap();
        // The unguarded copy first discriminates active child destruction by
        // its refunded pages, before an unsafe control can hit the guard probe.
        let mode = usize::from(guarded);
        send.prepare(
            source,
            |caller, cap, server| prepare_mode(caller, cap, server, mode),
            MemoryAttachments::single,
        )
        .unwrap();
        let amount = memory::budget::used(fixture.caller);
        let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let authority = crate::capability::node_admission_used();
        let metadata = node_budget::snapshot();
        let irq = crate::cpu::isa::lp::ops::get_int_state();
        if guarded {
            let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
            let _roots = ADDRESS_SPACE_TABLE.lock();
            let _ipc = IPC.write();
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _physical = PHYSICAL_FRAME_ALLOCATOR.lock();
            object::retirement_tests::with_registry_locked(|| {
                crate::capability::admission_tests::test_with_registry_locked(|| {
                    node_budget::with_locked(|| drop(send));
                });
            });
        } else {
            drop(send);
        }
        assert_eq!(
            memory::budget::used(fixture.caller),
            amount,
            "send abandonment released its child's backing"
        );
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
        assert_eq!(crate::capability::node_admission_used(), authority);
        assert_eq!(node_budget::snapshot(), metadata);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
        unlocked(&fixture); // Both original root leases remain retained.
        // Never reconstruct this abandoned operation or close its roots by ID.
    }
    crate::logln!(
        "[IPC scalar submission] copy/move sends and copy/move/read/write calls prepare outside \
         IPC with both roots; attachment/authority/endpoint rejection restores sources; heap-held \
         send publication and inert complete-send abandonment passed"
    );
}
