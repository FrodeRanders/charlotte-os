//! Scalar attachment/vector preparation and ordinary rollback run outside IPC.
//! Exact roots precede preparation; publication revalidates captured authority.
use super::*;
use crate::memory::{
    AddressSpaceHandle,
    KERNEL_ASID,
    object,
    operation::AddressSpaceOperation,
};

#[must_use]
struct Submission {
    caller: AddressSpaceId,
    connection: CapabilityId,
    payload: Capability,
    endpoint: EndpointId,
    server: AddressSpaceId,
    platform: bool,
    namespaces: [(AddressSpaceId, Option<AddressSpaceHandle>); 2],
    roots: [Option<AddressSpaceOperation>; 2],
}

impl Submission {
    fn prepare(
        caller: AddressSpaceId,
        connection: CapabilityId,
        needed: ConnectionRights,
    ) -> Result<Self, IpcError> {
        let trusted = crate::memory::budget::platform_identity(caller);
        let (payload, endpoint, server, namespaces) = {
            let ipc = IPC.read();
            let payload = ipc.cap(caller, connection)?;
            let Capability::Connection {
                endpoint,
                rights,
            } = payload
            else {
                return Err(IpcError::WrongType);
            };
            if !rights.contains(needed) {
                return Err(IpcError::PermissionDenied);
            }
            let server = reserve_endpoint_queue(&ipc, endpoint)?;
            let namespaces = [caller, server].map(|asid| {
                (asid, ipc.caps.get(&asid).and_then(|namespace| namespace.address_space))
            });
            (payload, endpoint, server, namespaces)
        };
        let mut owner = Self {
            caller,
            connection,
            payload,
            endpoint,
            server,
            platform: caller == KERNEL_ASID
                || trusted.is_some_and(|root| namespaces[0].1 == Some(root)),
            namespaces,
            roots: [None, None],
        };
        for (index, &(asid, handle)) in namespaces.iter().enumerate() {
            if asid == KERNEL_ASID || (index == 1 && asid == caller) {
                continue;
            }
            let root = handle.ok_or(IpcError::UnknownCapability).and_then(|handle| {
                AddressSpaceOperation::acquire(handle).map_err(|_| IpcError::Pending)
            });
            match root {
                Ok(root) => owner.roots[index] = Some(root),
                Err(error) => {
                    owner.complete()?;
                    return Err(error);
                }
            }
        }
        let validation = owner.validate(&IPC.read());
        if let Err(error) = validation {
            owner.complete()?;
            return Err(error);
        }
        Ok(owner)
    }

    fn validate(&self, ipc: &IpcRegistry) -> Result<(), IpcError> {
        for &(asid, handle) in &self.namespaces {
            if ipc.caps.get(&asid).and_then(|namespace| namespace.address_space) != handle {
                return Err(IpcError::UnknownCapability);
            }
        }
        if ipc.cap(self.caller, self.connection)? != self.payload {
            return Err(IpcError::UnknownCapability);
        }
        if reserve_endpoint_queue(ipc, self.endpoint)? != self.server {
            return Err(IpcError::UnknownCapability);
        }
        Ok(())
    }

    fn complete(self) -> Result<(), IpcError> {
        let mut error = None;
        for root in self.roots.into_iter().flatten() {
            if root.release().is_err() {
                error = Some(IpcError::UnknownCapability);
            }
        }
        error.map_or(Ok(()), Err)
    }

    fn run<T>(self, work: impl FnOnce(&Self) -> Result<T, IpcError>) -> Result<T, IpcError> {
        // Every ordinary result ends staging before root release. Panic retains
        // leases; no unknown destructor silently authorizes root teardown.
        let result = work(&self);
        self.complete()?;
        result
    }
}

pub(super) fn call_memory(
    caller: AddressSpaceId,
    connection: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory: MemoryObjectCap,
    delegation: Option<(CapabilityId, ConnectionRights)>,
    prepare: impl FnOnce(
        AddressSpaceId,
        MemoryObjectCap,
        AddressSpaceId,
    ) -> Result<object::PreparedTransfer, object::MemoryObjectError>,
) -> Result<CapabilityId, IpcError> {
    Submission::prepare(caller, connection, ConnectionRights::CALL)?.run(|submission| {
        PreparedCall::run(caller, connection, |prepared| {
            let delegated = {
                let mut ipc = IPC.write();
                submission.validate(&ipc)?;
                let delegated = match delegation {
                    Some((0, _)) => Some((0, ConnectionRights(0))),
                    Some((cap, rights)) => Some(mintable_endpoint(&ipc, caller, cap, rights)?),
                    None => None,
                };
                prepared.stage(&mut ipc, caller)?;
                if let Some((endpoint, rights)) = delegated {
                    PreparedConnection::prepare_into(
                        &mut prepared.connection,
                        &mut ipc,
                        caller,
                        submission.server,
                        endpoint,
                        rights,
                    )?;
                }
                delegated
            };
            prepared.attach(
                prepare(caller, memory, submission.server)
                    .map_err(|_| IpcError::MemoryTransferFailed)?,
            )?;
            let mut ipc = IPC.write();
            submission.validate(&ipc)?;
            if let Some((cap, rights)) = delegation
                && cap != 0
                && Some(mintable_endpoint(&ipc, caller, cap, rights)?) != delegated
            {
                return Err(IpcError::PermissionDenied);
            }
            prepared.commit(&mut ipc, submission.endpoint, opcode, arg0)
        })
    })
}

/// One containing owner for source/destination roots, escrow/private backing,
/// attachment storage and post-publication delivery. Drop retains all fields;
/// only explicit finish may invoke the active transfer cleanup policy.
#[must_use]
struct PreparedSend(core::mem::ManuallyDrop<SendResources>);

struct SendResources {
    submission: Option<Submission>,
    transfer: Option<object::PreparedTransfer>,
    memory: Option<MemoryAttachments>,
    delivery: Option<Delivery>,
}

impl PreparedSend {
    fn new(sender: AddressSpaceId, connection: CapabilityId) -> Result<Self, IpcError> {
        let submission = Submission::prepare(sender, connection, ConnectionRights::SEND)?;
        Ok(Self(core::mem::ManuallyDrop::new(SendResources {
            submission: Some(submission),
            transfer: None,
            memory: None,
            delivery: None,
        })))
    }

    fn prepare(
        &mut self,
        cap: MemoryObjectCap,
        prepare: impl FnOnce(
            AddressSpaceId,
            MemoryObjectCap,
            AddressSpaceId,
        ) -> Result<object::PreparedTransfer, object::MemoryObjectError>,
        attachments: impl FnOnce(bool, MemoryObjectCap) -> Result<MemoryAttachments, IpcError>,
    ) -> Result<(), IpcError> {
        let resources = &mut *self.0;
        assert!(resources.transfer.is_none() && resources.memory.is_none());
        let submission = resources.submission.as_ref().unwrap();
        resources.transfer = Some(
            prepare(submission.caller, cap, submission.server)
                .map_err(|_| IpcError::MemoryTransferFailed)?,
        );
        resources.memory = Some(attachments(
            submission.platform,
            resources.transfer.as_ref().unwrap().target_cap(),
        )?);
        Ok(())
    }

    fn publish(&mut self, opcode: u32, arg0: u64) -> Result<(), IpcError> {
        let resources = &mut *self.0;
        let submission = resources.submission.as_ref().unwrap();
        let mut ipc = IPC.write();
        submission.validate(&ipc)?;
        object::commit_undelivered_transfers(core::slice::from_mut(
            resources.transfer.as_mut().unwrap(),
        ))
        .map_err(|_| IpcError::MemoryTransferFailed)?;
        resources.delivery = Some(
            enqueue_message(
                &mut ipc,
                submission.endpoint,
                submission.caller,
                opcode,
                arg0,
                None,
                resources.memory.take().unwrap(),
                None,
            )
            .expect("reserved memory-send endpoint changed under IPC ownership"),
        );
        Ok(())
    }

    fn finish(mut self) -> Result<(), IpcError> {
        // All ordinary returns leave IPC before cleanup. Keep both exact roots
        // through source fence/pin completion, storage disposal and delivery.
        drop(self.0.transfer.take());
        if let Some(memory) = self.0.memory.take() {
            memory.finish();
        }
        if let Some(delivery) = self.0.delivery.take() {
            delivery.notify();
        }
        self.0.submission.take().unwrap().complete()
    }
}

impl Drop for PreparedSend {
    fn drop(&mut self) {
        // No implicit field destruction, registry access, source rollback,
        // backing release, notification or root completion on abandonment.
    }
}

pub(super) fn send_memory(
    sender: AddressSpaceId,
    connection: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory: MemoryObjectCap,
    prepare: impl FnOnce(
        AddressSpaceId,
        MemoryObjectCap,
        AddressSpaceId,
    ) -> Result<object::PreparedTransfer, object::MemoryObjectError>,
) -> Result<(), IpcError> {
    let mut operation = PreparedSend::new(sender, connection)?;
    let result = operation
        .prepare(memory, prepare, MemoryAttachments::single)
        .and_then(|()| operation.publish(opcode, arg0));
    operation.finish()?;
    result
}

/// Vector staging owns every partial transfer, metadata buffer and exact root.
/// Explicit finish leaves IPC before child cleanup; fallback retains all fields.
#[must_use]
struct PreparedVectorSend(core::mem::ManuallyDrop<VectorSendResources>);
struct VectorSendResources {
    submission: Option<Submission>,
    transfers: Buffer<object::PreparedTransfer>,
    caps: Buffer<MemoryObjectCap>,
    memory: Option<MemoryAttachments>,
    delivery: Option<Delivery>,
}
impl PreparedVectorSend {
    fn new(submission: Submission) -> Self {
        let platform = submission.platform;
        Self(core::mem::ManuallyDrop::new(VectorSendResources {
            submission: Some(submission),
            transfers: buffer(platform),
            caps: buffer(platform),
            memory: None,
            delivery: None,
        }))
    }

    fn prepare(&mut self, descriptor: MemoryObjectCap) -> Result<(), IpcError> {
        let resources = &mut *self.0;
        let submission = resources.submission.as_ref().unwrap();
        read_vector_page(
            submission.caller,
            descriptor,
            submission.server,
            false,
            submission.platform,
            &mut resources.caps,
            &mut resources.transfers,
        )?;
        resources.memory = Some(MemoryAttachments::try_new(take_buffer(&mut resources.caps))?);
        Ok(())
    }

    fn publish(&mut self, opcode: u32, arg0: u64) -> Result<(), IpcError> {
        let resources = &mut *self.0;
        let submission = resources.submission.as_ref().unwrap();
        let mut ipc = IPC.write();
        submission.validate(&ipc)?;
        object::commit_undelivered_transfers(&mut resources.transfers)
            .map_err(|_| IpcError::MemoryTransferFailed)?;
        resources.delivery = Some(
            enqueue_message(
                &mut ipc,
                submission.endpoint,
                submission.caller,
                opcode,
                arg0,
                None,
                resources.memory.take().unwrap(),
                None,
            )
            .expect("reserved vector-send endpoint changed under IPC ownership"),
        );
        Ok(())
    }

    fn finish(mut self, descriptor: Option<MemoryObjectCap>) -> Result<(), IpcError> {
        drop(take_buffer(&mut self.0.transfers));
        drop(take_buffer(&mut self.0.caps));
        if let Some(memory) = self.0.memory.take() {
            memory.finish();
        }
        if let Some(delivery) = self.0.delivery.take() {
            delivery.notify();
        }
        let submission = self.0.submission.take().unwrap();
        if let Some(descriptor) = descriptor {
            let _ = object::close_cap(submission.caller, descriptor);
        }
        submission.complete()
    }
}

pub(super) fn vector(
    caller: AddressSpaceId,
    connection: CapabilityId,
    opcode: u32,
    arg0: u64,
    descriptor: MemoryObjectCap,
    call: bool,
) -> Result<Option<CapabilityId>, IpcError> {
    let submission = Submission::prepare(
        caller,
        connection,
        if call {
            ConnectionRights::CALL
        } else {
            ConnectionRights::SEND
        },
    )?;
    if call {
        submission.run(|submission| {
            let cap = PreparedCall::run(caller, connection, |prepared| {
                {
                    let mut ipc = IPC.write();
                    submission.validate(&ipc)?;
                    prepared.stage(&mut ipc, caller)?;
                }
                let resources = &mut **prepared;
                read_vector_page(
                    caller,
                    descriptor,
                    submission.server,
                    true,
                    resources.platform,
                    &mut resources.memory,
                    &mut resources.transfers,
                )?;
                prepared.attach_vector()?;
                let mut ipc = IPC.write();
                submission.validate(&ipc)?;
                prepared.commit(&mut ipc, submission.endpoint, opcode, arg0)
            })?;
            let _ = object::close_cap(caller, descriptor);
            Ok(Some(cap))
        })
    } else {
        let mut operation = PreparedVectorSend::new(submission);
        let result = operation.prepare(descriptor).and_then(|()| operation.publish(opcode, arg0));
        operation.finish(result.is_ok().then_some(descriptor))?;
        result.map(|()| None)
    }
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(super) mod tests;
