//! Own a borrowed-memory reply across the IPC-unlocked invalidation interval.
//! Returned connections keep their minting source claimed through publication.
//! Returned memory owns source escrow/backing through joint publication.
//! Explicit cancellation has a separate owner; bulk cleanup remains serialized.

use super::*;
use crate::memory::{
    AddressSpaceHandle,
    KERNEL_ASID,
    object::{
        LoanRevocation,
        MemoryObjectError,
    },
    operation::AddressSpaceOperation,
};

pub(crate) mod tests;

/// A published claim is never automatically cleared by Drop. Its roots,
/// backing pins and Revoking loan fences survive an abandoned operation.
#[must_use]
struct PreparedReply {
    server: AddressSpaceId,
    cap: CapabilityId,
    token: ReplyTokenId,
    call: PendingCallId,
    identities: [(AddressSpaceId, Option<AddressSpaceHandle>); 2],
    namespaces: [Option<AddressSpaceOperation>; 2],
    loans: Vec<(MemoryBorrow, LoanRevocation)>,
    connection: Option<ReturnedConnection>,
    memory: Option<crate::memory::object::PreparedTransfer>,
}

/// The source is borrowed authority, retained by the published reply claim.
/// Its existing IPC payload keeps the endpoint alive; no scalar adoption/move
/// or authority restoration is needed. The destination remains unpublished.
struct ReturnedConnection {
    source: CapabilityId,
    grant: PreparedConnection,
}

impl PreparedReply {
    fn prepare(server: AddressSpaceId, cap: CapabilityId) -> Result<Self, IpcError> {
        Self::prepare_with_connection(server, cap, None)
    }

    fn prepare_with_connection(
        server: AddressSpaceId,
        cap: CapabilityId,
        returned: Option<(CapabilityId, ConnectionRights)>,
    ) -> Result<Self, IpcError> {
        Self::prepare_with_outputs(server, cap, returned, None)
    }

    fn prepare_with_memory(
        server: AddressSpaceId,
        cap: CapabilityId,
        source: MemoryObjectCap,
    ) -> Result<Self, IpcError> {
        Self::prepare_with_outputs(server, cap, None, Some(source))
    }

    fn prepare_with_outputs(
        server: AddressSpaceId,
        cap: CapabilityId,
        returned: Option<(CapabilityId, ConnectionRights)>,
        memory: Option<MemoryObjectCap>,
    ) -> Result<Self, IpcError> {
        let (token, call, identities) = {
            let ipc = IPC.read();
            let (token, call, caller) = validate(&ipc, server, cap)?;
            let identity = |asid| (asid, ipc.caps.get(&asid).and_then(|caps| caps.address_space));
            (token, call, [identity(caller), identity(server)])
        };
        let mut operation = Self {
            server,
            cap,
            token,
            call,
            identities,
            namespaces: [None, None],
            loans: Vec::new(),
            connection: None,
            memory: None,
        };
        let prepared = (|| {
            // Capture before taking IPC. Acquiring lifecycle beneath IPC would
            // deadlock namespace cleanup. Kernel identity cannot be recycled.
            for (index, (asid, handle)) in identities.into_iter().enumerate() {
                if asid != KERNEL_ASID {
                    let handle = handle.ok_or(IpcError::UnknownCapability)?;
                    operation.namespaces[index] = Some(
                        AddressSpaceOperation::acquire(handle)
                            .map_err(|_| IpcError::ResourceLimit)?,
                    );
                }
            }
            let mut ipc = IPC.write();
            let (current_token, current_call, _) = validate(&ipc, server, cap)?;
            if (current_token, current_call) != (token, call)
                || identities.iter().any(|(asid, handle)| {
                    ipc.caps.get(asid).and_then(|caps| caps.address_space) != *handle
                })
            {
                return Err(IpcError::UnknownCapability);
            }
            if let Some((source, requested)) = returned {
                let (endpoint, rights) = mintable_endpoint(&ipc, server, source, requested)?;
                // Ordinary lookup rejects undelivered sources. Delivered
                // identities remain protected against close by this reply claim.
                operation.connection = Some(ReturnedConnection {
                    source,
                    grant: PreparedConnection::new(
                        &mut ipc,
                        identities[0].0,
                        identities[0].0,
                        endpoint,
                        rights,
                    )?,
                });
            }
            if let Some(source) = memory {
                if memory_source_reclaimable(&ipc, server, source) {
                    return Err(IpcError::Pending);
                }
                operation.memory = Some(
                    crate::memory::object::prepare_move(server, source, identities[0].0)
                        .map_err(|_| IpcError::MemoryTransferFailed)?,
                );
            }
            let borrows = &ipc.reply_tokens[&token].borrows;
            if borrows.len() > CAP_VECTOR_MAX {
                return Err(IpcError::ResourceLimit);
            }
            operation
                .loans
                .try_reserve_exact(borrows.len())
                .map_err(|_| IpcError::ResourceLimit)?;
            for &borrow in borrows {
                if borrow.owner != identities[0].0 || borrow.borrower != server {
                    operation.cancel_prepared();
                    return Err(IpcError::MemoryTransferFailed);
                }
                match LoanRevocation::prepare(
                    borrow.owner,
                    borrow.owner_cap,
                    borrow.borrower,
                    borrow.borrower_cap,
                ) {
                    Ok(loan) => operation.loans.push((borrow, loan)),
                    Err(_) => {
                        operation.cancel_prepared();
                        return Err(IpcError::MemoryTransferFailed);
                    }
                }
            }
            // No fallible work remains before publishing this ownership claim.
            ipc.reply_tokens.get_mut(&token).unwrap().completing = true;
            ipc.reply_tokens.get_mut(&token).unwrap().connection_source =
                operation.connection.as_ref().map(|connection| connection.source);
            Ok(())
        })();
        if let Err(error) = prepared {
            drop(operation.memory.take());
            drop(operation.connection.take());
            operation.release_namespaces()?;
            return Err(error);
        }
        Ok(operation)
    }

    fn cancel_prepared(&mut self) {
        while let Some((_, loan)) = self.loans.pop() {
            loan.cancel_prepared();
        }
    }

    fn finish(self, result: i64) -> Result<(), IpcError> {
        self.finish_with(result, LoanRevocation::finish)
    }

    fn finish_with(
        self,
        result: i64,
        finish: impl FnMut(LoanRevocation) -> Result<(), MemoryObjectError>,
    ) -> Result<(), IpcError> {
        self.finish_with_publication(result, finish, |memory, authority| {
            crate::memory::object::commit_undelivered_transfers_with_authority(memory, authority)
                .map_err(|_| IpcError::MemoryTransferFailed)
        })
    }

    fn finish_with_publication(
        mut self,
        result: i64,
        mut finish: impl FnMut(LoanRevocation) -> Result<(), MemoryObjectError>,
        publish: impl FnOnce(
            &mut [crate::memory::object::PreparedTransfer],
            &mut [&mut crate::capability::Reservation],
        ) -> Result<(), IpcError>,
    ) -> Result<(), IpcError> {
        while let Some((borrow, loan)) = self.loans.pop() {
            // No IPC/lifecycle/table/registry guard crosses this callback.
            if finish(loan).is_err() {
                let mut ipc = IPC.write();
                self.abort_prepared(&mut ipc);
                drop(ipc);
                self.release_namespaces()?;
                return Err(IpcError::MemoryTransferFailed);
            }
            let mut ipc = IPC.write();
            let token = ipc.reply_tokens.get_mut(&self.token).expect("claimed reply missing");
            assert!(token.completing);
            assert_eq!(token.borrows.pop(), Some(borrow));
        }
        let mut ipc = IPC.write();
        if let Some(connection) = self.connection.as_mut() {
            assert_eq!(ipc.reply_tokens[&self.token].connection_source, Some(connection.source));
            // Source close cannot consume this borrowed capability until the
            // claim ends. Publication and payload install share IPC exclusion.
            assert!(
                ipc.cap(self.server, connection.source).is_ok(),
                "claimed minting source missing"
            );
        }
        let published = if let Some(connection) = self.connection.as_mut() {
            publish(self.memory.as_mut_slice(), &mut [&mut connection.grant.authority])
        } else if self.memory.is_some() {
            publish(self.memory.as_mut_slice(), &mut [])
        } else {
            Ok(())
        };
        if let Err(error) = published {
            self.abort_prepared(&mut ipc);
            drop(ipc);
            self.release_namespaces()?;
            return Err(error);
        }
        let returned_memory = self.memory.as_ref().map(|transfer| transfer.target_cap());
        drop(self.memory.take());
        let returned_cap =
            self.connection.take().map(|connection| connection.grant.install(&mut ipc));
        let token = ipc.reply_tokens.remove(&self.token).expect("claimed reply missing");
        assert!(token.completing && token.borrows.is_empty());
        assert_eq!(token.call, self.call);
        assert_eq!(ipc.caps[&self.server].address_space, self.identities[1].1);
        let call = ipc.pending_calls.get_mut(&self.call).expect("claimed call missing");
        assert_eq!(call.caller, self.identities[0].0);
        assert!(call.result.is_none());
        call.result = Some(ReplyValue {
            result,
            cap: returned_cap,
            memory: returned_memory,
        });
        let observers = call.observers.close();
        ipc.remove_cap(self.server, self.cap).expect("claimed reply cap missing");
        drop(ipc);
        let released = self.release_namespaces();
        signal_observers(observers);
        released
    }

    fn release_namespaces(self) -> Result<(), IpcError> {
        assert!(self.loans.is_empty(), "releasing roots before prepared loans finish");
        assert!(self.connection.is_none(), "releasing roots before returned authority cleanup");
        assert!(self.memory.is_none(), "releasing roots before returned memory cleanup");
        let mut error = None;
        for namespace in self.namespaces.into_iter().flatten() {
            if namespace.release().is_err() {
                error = Some(IpcError::UnknownCapability);
            }
        }
        error.map_or(Ok(()), Err)
    }

    /// Retain failed loan backing, but restore every unstarted receipt and
    /// refund unpublished destination authority before releasing root leases.
    fn abort_prepared(&mut self, ipc: &mut IpcRegistry) {
        self.cancel_prepared();
        drop(self.memory.take());
        drop(self.connection.take());
        let token = ipc.reply_tokens.get_mut(&self.token).expect("claimed reply missing");
        assert!(token.completing);
        token.connection_source = None;
        token.completing = false;
    }
}

fn memory_source_reclaimable(
    ipc: &IpcRegistry,
    server: AddressSpaceId,
    source: MemoryObjectCap,
) -> bool {
    ipc.pending_calls.values().any(|call| {
        call.caller == server
            && !call.observed
            && call.result.is_some_and(|result| result.memory == Some(source))
    }) || ipc.endpoints.values().any(|endpoint| {
        endpoint.owner == server
            && endpoint.queue.iter().any(|message| message.memory.contains(&source))
    })
}

fn validate(
    ipc: &IpcRegistry,
    server: AddressSpaceId,
    cap: CapabilityId,
) -> Result<(ReplyTokenId, PendingCallId, AddressSpaceId), IpcError> {
    let token_id = match ipc.cap(server, cap)? {
        Capability::ReplyToken {
            token,
        } => token,
        _ => return Err(IpcError::WrongType),
    };
    let token = ipc.reply_tokens.get(&token_id).ok_or(IpcError::UnknownCapability)?;
    if token.server != server {
        return Err(IpcError::PermissionDenied);
    }
    if token.completing {
        return Err(IpcError::ReplyAlreadyUsed);
    }
    if token.cleanup_failed {
        return Err(IpcError::MemoryTransferFailed);
    }
    let call = ipc.pending_calls.get(&token.call).ok_or(IpcError::UnknownCapability)?;
    if call.result.is_some() {
        return Err(IpcError::ReplyAlreadyUsed);
    }
    Ok((token_id, token.call, call.caller))
}

pub(super) fn complete(
    server: AddressSpaceId,
    cap: CapabilityId,
    result: i64,
) -> Result<(), IpcError> {
    PreparedReply::prepare(server, cap)?.finish(result)
}

pub(super) fn complete_with_connection(
    server: AddressSpaceId,
    cap: CapabilityId,
    source: CapabilityId,
    rights: ConnectionRights,
    result: i64,
) -> Result<(), IpcError> {
    PreparedReply::prepare_with_connection(server, cap, Some((source, rights)))?.finish(result)
}

pub(super) fn complete_with_memory(
    server: AddressSpaceId,
    cap: CapabilityId,
    source: MemoryObjectCap,
    result: i64,
) -> Result<(), IpcError> {
    PreparedReply::prepare_with_memory(server, cap, source)?.finish(result)
}
