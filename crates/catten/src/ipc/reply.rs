//! Own a borrowed-memory reply across the IPC-unlocked invalidation interval.
//! Returned-authority replies and cancellation retain their serialized adapter.

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
}

impl PreparedReply {
    fn prepare(server: AddressSpaceId, cap: CapabilityId) -> Result<Self, IpcError> {
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
            ipc.reply_tokens.get_mut(&token).unwrap().replying = true;
            Ok(())
        })();
        if let Err(error) = prepared {
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
        mut self,
        result: i64,
        mut finish: impl FnMut(LoanRevocation) -> Result<(), MemoryObjectError>,
    ) -> Result<(), IpcError> {
        while let Some((borrow, loan)) = self.loans.pop() {
            // No IPC/lifecycle/table/registry guard crosses this callback.
            if finish(loan).is_err() {
                let mut ipc = IPC.write();
                self.cancel_prepared();
                ipc.reply_tokens.get_mut(&self.token).expect("claimed reply missing").replying =
                    false;
                drop(ipc);
                self.release_namespaces()?;
                return Err(IpcError::MemoryTransferFailed);
            }
            let mut ipc = IPC.write();
            let token = ipc.reply_tokens.get_mut(&self.token).expect("claimed reply missing");
            assert!(token.replying);
            assert_eq!(token.borrows.pop(), Some(borrow));
        }
        let mut ipc = IPC.write();
        let token = ipc.reply_tokens.remove(&self.token).expect("claimed reply missing");
        assert!(token.replying && token.borrows.is_empty());
        assert_eq!(token.call, self.call);
        assert_eq!(ipc.caps[&self.server].address_space, self.identities[1].1);
        let call = ipc.pending_calls.get_mut(&self.call).expect("claimed call missing");
        assert_eq!(call.caller, self.identities[0].0);
        assert!(call.result.is_none());
        call.result = Some(ReplyValue {
            result,
            cap: None,
            memory: None,
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
        let mut error = None;
        for namespace in self.namespaces.into_iter().flatten() {
            if namespace.release().is_err() {
                error = Some(IpcError::UnknownCapability);
            }
        }
        error.map_or(Ok(()), Err)
    }
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
    if token.replying {
        return Err(IpcError::ReplyAlreadyUsed);
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
