//! Explicit call/reply close owns loan cleanup outside IPC serialization.
//! Endpoint and whole-domain bulk cleanup retain their serialized adapter.

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum CloseTarget {
    Call,
    Reply,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    target: CloseTarget,
    token: ReplyTokenId,
    call: PendingCallId,
    caller: AddressSpaceId,
    server: AddressSpaceId,
    namespaces: [Option<AddressSpaceHandle>; 2],
}

/// Records, queue attachments and exact roots remain retained by the claim.
/// Abandonment retains that claim, the leases and every uncertain loan pin.
#[must_use]
struct PreparedCancellation {
    asid: AddressSpaceId,
    cap: CapabilityId,
    identity: Identity,
    namespaces: [Option<AddressSpaceOperation>; 2],
    loans: Vec<(MemoryBorrow, LoanRevocation)>,
}

fn resolve(
    ipc: &IpcRegistry,
    asid: AddressSpaceId,
    cap: CapabilityId,
) -> Result<Option<Identity>, IpcError> {
    let (target, token) = match ipc.cap(asid, cap)? {
        Capability::PendingCall {
            call,
        } => {
            let pending = ipc.pending_calls.get(&call).ok_or(IpcError::UnknownCapability)?;
            if pending.caller != asid {
                return Err(IpcError::PermissionDenied);
            }
            if pending.result.is_some() {
                return Ok(None);
            }
            let token = ipc
                .reply_tokens
                .iter()
                .find_map(|(&id, token)| (token.call == call).then_some(id))
                .ok_or(IpcError::UnknownCapability)?;
            (CloseTarget::Call, token)
        }
        Capability::ReplyToken {
            token,
        } => (CloseTarget::Reply, token),
        _ => return Ok(None),
    };
    let reply = ipc.reply_tokens.get(&token).ok_or(IpcError::UnknownCapability)?;
    if target == CloseTarget::Reply && reply.server != asid {
        return Err(IpcError::PermissionDenied);
    }
    if reply.completing {
        return Err(IpcError::Pending);
    }
    if reply.cleanup_failed {
        return Err(IpcError::MemoryTransferFailed);
    }
    let pending = ipc.pending_calls.get(&reply.call).ok_or(IpcError::UnknownCapability)?;
    if pending.result.is_some() {
        return Err(IpcError::ReplyAlreadyUsed);
    }
    Ok(Some(Identity {
        target,
        token,
        call: reply.call,
        caller: pending.caller,
        server: reply.server,
        namespaces: [pending.caller, reply.server]
            .map(|owner| ipc.caps.get(&owner).and_then(|caps| caps.address_space)),
    }))
}

pub(super) fn has_loans(
    ipc: &IpcRegistry,
    asid: AddressSpaceId,
    cap: CapabilityId,
) -> Result<bool, IpcError> {
    Ok(resolve(ipc, asid, cap)?
        .is_some_and(|identity| !ipc.reply_tokens[&identity.token].borrows.is_empty()))
}

impl PreparedCancellation {
    fn prepare(asid: AddressSpaceId, cap: CapabilityId) -> Result<Self, IpcError> {
        let identity = resolve(&IPC.read(), asid, cap)?.ok_or(IpcError::Pending)?;
        let mut operation = Self {
            asid,
            cap,
            identity,
            namespaces: [None, None],
            loans: Vec::new(),
        };
        let prepared = (|| {
            // Lifecycle always precedes IPC. Kernel identity is permanent.
            for (index, owner) in [identity.caller, identity.server].into_iter().enumerate() {
                if owner != KERNEL_ASID {
                    let handle = identity.namespaces[index].ok_or(IpcError::UnknownCapability)?;
                    operation.namespaces[index] = Some(
                        AddressSpaceOperation::acquire(handle)
                            .map_err(|_| IpcError::ResourceLimit)?,
                    );
                }
            }
            let mut ipc = IPC.write();
            if resolve(&ipc, asid, cap)? != Some(identity) {
                return Err(IpcError::Pending);
            }
            let borrows = &ipc.reply_tokens[&identity.token].borrows;
            if borrows.len() > CAP_VECTOR_MAX {
                return Err(IpcError::ResourceLimit);
            }
            operation
                .loans
                .try_reserve_exact(borrows.len())
                .map_err(|_| IpcError::ResourceLimit)?;
            for &borrow in borrows {
                if borrow.owner != identity.caller || borrow.borrower != identity.server {
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
            ipc.reply_tokens.get_mut(&identity.token).unwrap().completing = true;
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

    fn release_namespaces(self) -> Result<(), IpcError> {
        assert!(self.loans.is_empty(), "releasing cancellation roots before loans finish");
        let mut error = None;
        for namespace in self.namespaces.into_iter().flatten() {
            if namespace.release().is_err() {
                error = Some(IpcError::UnknownCapability);
            }
        }
        error.map_or(Ok(()), Err)
    }

    fn finish_with(
        mut self,
        mut finish: impl FnMut(LoanRevocation) -> Result<(), MemoryObjectError>,
    ) -> Result<(), IpcError> {
        while let Some((borrow, loan)) = self.loans.pop() {
            // No lifecycle, IPC, registry or table guard crosses cleanup.
            if finish(loan).is_err() {
                let mut ipc = IPC.write();
                self.cancel_prepared();
                let token = ipc
                    .reply_tokens
                    .get_mut(&self.identity.token)
                    .expect("claimed cancellation missing");
                assert!(token.completing);
                token.completing = false;
                token.cleanup_failed = true;
                drop(ipc);
                self.release_namespaces()?;
                // Capability and pending record are NOT consumed; no terminal
                // result is published while a loan's cleanup is uncertain.
                return Err(IpcError::MemoryTransferFailed);
            }
            let mut ipc = IPC.write();
            let token = ipc
                .reply_tokens
                .get_mut(&self.identity.token)
                .expect("claimed cancellation missing");
            assert!(token.completing);
            assert_eq!(token.borrows.pop(), Some(borrow));
        }
        let mut ipc = IPC.write();
        let identity = self.identity;
        assert_eq!(ipc.caps[&identity.caller].address_space, identity.namespaces[0]);
        assert_eq!(ipc.caps[&identity.server].address_space, identity.namespaces[1]);
        let token = ipc.reply_tokens.remove(&identity.token).expect("claimed cancellation missing");
        assert!(token.completing && token.borrows.is_empty());
        assert_eq!(token.call, identity.call);
        // A queued cancellation leaves the message in place until every loan
        // completes. Receive and endpoint-close must respect its claim.
        let queued = ipc.endpoints.values_mut().find_map(|endpoint| {
            if endpoint.owner != identity.server {
                return None;
            }
            let index =
                endpoint.queue.iter().position(|message| message.reply == Some(identity.token))?;
            let message = endpoint.queue.remove(index)?;
            // Removing a claimed front may expose work whose original CQ wake
            // was consumed while receive could not yet dequeue it.
            Some((
                message,
                Delivery {
                    observers: endpoint.readiness_observers.drain(),
                    cq_wake: endpoint.notify_cq.map(|cq| (endpoint.owner, cq)),
                },
            ))
        });
        let delivery = queued.map(|(message, delivery)| {
            for cap in message.memory {
                let _ = crate::memory::object::try_close_cap(identity.server, cap);
            }
            if let Some(cap) = message.connection {
                let _ = ipc.remove_cap(identity.server, cap);
            }
            delivery
        });
        // Receive mints at most one visible cap for this one-shot token. Find
        // it without allocating a teardown snapshot or fresh authority.
        let reply_cap = ipc.caps.get(&identity.server).and_then(|caps| {
            caps.caps.iter().find_map(|(&id, cap)| {
                (cap.payload
                    == Capability::ReplyToken {
                        token: identity.token,
                    })
                .then_some(id)
            })
        });
        if let Some(cap) = reply_cap {
            ipc.remove_cap(identity.server, cap).expect("claimed reply cap missing");
        }
        let observers = match identity.target {
            CloseTarget::Call => {
                ipc.remove_cap(self.asid, self.cap).expect("claimed call cap missing");
                ipc.pending_calls
                    .remove(&identity.call)
                    .expect("claimed pending call missing")
                    .observers
                    .close()
            }
            CloseTarget::Reply => {
                assert_eq!(reply_cap, Some(self.cap));
                let call = ipc
                    .pending_calls
                    .get_mut(&identity.call)
                    .expect("claimed pending call missing");
                assert!(call.result.is_none());
                call.result = Some(ReplyValue {
                    result: REPLY_CANCELLED,
                    cap: None,
                    memory: None,
                });
                call.observers.close()
            }
        };
        drop(ipc);
        let released = self.release_namespaces();
        if let Some(delivery) = delivery {
            deliver(delivery);
        }
        signal_observers(observers);
        released
    }
}

pub(super) fn close(asid: AddressSpaceId, cap: CapabilityId) -> Result<(), IpcError> {
    PreparedCancellation::prepare(asid, cap)?.finish_with(LoanRevocation::finish)
}
