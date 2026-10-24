//! Copy/vector preparation and every unpublished rollback run outside IPC.
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
    namespaces: [(AddressSpaceId, Option<AddressSpaceHandle>); 2],
    roots: [Option<AddressSpaceOperation>; 2],
}

impl Submission {
    fn prepare(
        caller: AddressSpaceId,
        connection: CapabilityId,
        needed: ConnectionRights,
    ) -> Result<Self, IpcError> {
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

pub(super) fn call_copy(
    caller: AddressSpaceId,
    connection: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory: MemoryObjectCap,
    delegation: Option<(CapabilityId, ConnectionRights)>,
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
                ipc.stage_call(caller, prepared)?;
                if let Some((endpoint, rights)) = delegated {
                    prepared.connection = Some(PreparedConnection::new(
                        &mut ipc,
                        caller,
                        submission.server,
                        endpoint,
                        rights,
                    )?);
                }
                delegated
            };
            prepared.attach(
                object::prepare_copy(caller, memory, submission.server)
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

pub(super) fn send_copy(
    sender: AddressSpaceId,
    connection: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory: MemoryObjectCap,
) -> Result<(), IpcError> {
    Submission::prepare(sender, connection, ConnectionRights::SEND)?.run(|submission| {
        let mut transfer = object::prepare_copy(sender, memory, submission.server)
            .map_err(|_| IpcError::MemoryTransferFailed)?;
        let memory = MemoryAttachments::single(transfer.target_cap())?;
        let delivery = {
            let mut ipc = IPC.write();
            submission.validate(&ipc)?;
            object::commit_undelivered_transfers(core::slice::from_mut(&mut transfer))
                .map_err(|_| IpcError::MemoryTransferFailed)?;
            enqueue_message(&mut ipc, submission.endpoint, sender, opcode, arg0, None, memory, None)
                .expect("reserved copy-send endpoint changed under IPC ownership")
        };
        drop(transfer);
        deliver(delivery);
        Ok(())
    })
}

pub(super) fn vector(
    caller: AddressSpaceId,
    connection: CapabilityId,
    opcode: u32,
    arg0: u64,
    descriptor: MemoryObjectCap,
    call: bool,
) -> Result<Option<CapabilityId>, IpcError> {
    Submission::prepare(
        caller,
        connection,
        if call {
            ConnectionRights::CALL
        } else {
            ConnectionRights::SEND
        },
    )?
    .run(|submission| {
        let result = if call {
            Some(PreparedCall::run(caller, connection, |prepared| {
                {
                    let mut ipc = IPC.write();
                    submission.validate(&ipc)?;
                    ipc.stage_call(caller, prepared)?;
                }
                let mut caps = Vec::new();
                let transfers =
                    read_vector_page(caller, descriptor, submission.server, true, &mut caps)?;
                prepared.attach_vector(transfers, caps)?;
                let mut ipc = IPC.write();
                submission.validate(&ipc)?;
                prepared.commit(&mut ipc, submission.endpoint, opcode, arg0)
            })?)
        } else {
            let mut caps = Vec::new();
            let mut transfers =
                read_vector_page(caller, descriptor, submission.server, false, &mut caps)?;
            let memory = MemoryAttachments::try_new(caps)?;
            let delivery = {
                let mut ipc = IPC.write();
                submission.validate(&ipc)?;
                object::commit_undelivered_transfers(&mut transfers)
                    .map_err(|_| IpcError::MemoryTransferFailed)?;
                enqueue_message(
                    &mut ipc,
                    submission.endpoint,
                    caller,
                    opcode,
                    arg0,
                    None,
                    memory,
                    None,
                )
                .expect("reserved vector-send endpoint changed under IPC ownership")
            };
            drop(transfers);
            deliver(delivery);
            None
        };
        let _ = object::close_cap(caller, descriptor);
        Ok(result)
    })
}

pub(super) mod tests;
