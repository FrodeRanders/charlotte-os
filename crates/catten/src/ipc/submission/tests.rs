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
                let mut prepared = IPC.write().stage_call(fixture.caller.id()).unwrap();
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
                    prepared.commit_retained(&mut ipc, submission.endpoint, 1, 0, |caps| {
                        if publication {
                            MemoryAttachments::try_new(caps)
                        } else {
                            Err(IpcError::ResourceLimit)
                        }
                    })
                };
                let (error, owner) = result.err().expect("fault publication succeeded");
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
                drop(owner);
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
