//! Explicit call/reply close owns loan cleanup outside IPC serialization.
//! Explicit endpoint close borrows its server-root owner for one queued call
//! at a time. Namespace retirement borrows a closing root and admits peer
//! cleanup leases before claiming the token, including already-closing peers.

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
    Endpoint(EndpointId),
    Namespace(AddressSpaceId),
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

#[derive(Clone, Copy)]
enum RootOwner<'a> {
    Endpoint(&'a endpoint_close::PreparedEndpointClose),
    Namespace(&'a crate::memory::retirement::ClosingAddressSpace),
}

/// Records, queue attachments and exact roots remain retained by the claim.
/// Abandonment retains that claim, the leases and every uncertain loan pin.
#[must_use]
pub(super) struct PreparedCancellation<'a> {
    asid: AddressSpaceId,
    cap: CapabilityId,
    identity: Identity,
    namespaces: [Option<AddressSpaceOperation>; 2],
    loans: Vec<(MemoryBorrow, LoanRevocation)>,
    root_owner: Option<RootOwner<'a>>,
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
    if let Capability::Endpoint {
        endpoint,
        ..
    } = ipc.cap(asid, cap)?
    {
        return Ok(ipc.endpoints.get(&endpoint).is_some_and(|endpoint| {
            endpoint.owner == asid
                && endpoint.queue.iter().any(|message| {
                    message.reply.is_some_and(|token| {
                        ipc.reply_tokens.get(&token).is_some_and(|token| !token.borrows.is_empty())
                    })
                })
        }));
    }
    Ok(resolve(ipc, asid, cap)?
        .is_some_and(|identity| !ipc.reply_tokens[&identity.token].borrows.is_empty()))
}

fn resolve_endpoint_front(
    ipc: &IpcRegistry,
    asid: AddressSpaceId,
    cap: CapabilityId,
) -> Result<Option<Identity>, IpcError> {
    let Capability::Endpoint {
        endpoint,
        ..
    } = ipc.cap(asid, cap)?
    else {
        return Err(IpcError::WrongType);
    };
    let record = ipc.endpoints.get(&endpoint).ok_or(IpcError::UnknownCapability)?;
    if record.owner != asid || !record.closing || record.closed {
        return Err(IpcError::PermissionDenied);
    }
    let token = record.queue.front().and_then(|message| message.reply).ok_or(IpcError::Pending)?;
    let reply = ipc.reply_tokens.get(&token).ok_or(IpcError::UnknownCapability)?;
    if reply.server != asid {
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
    assert_eq!(record.queue.front().unwrap().sender, pending.caller);
    Ok(Some(Identity {
        target: CloseTarget::Endpoint(endpoint),
        token,
        call: reply.call,
        caller: pending.caller,
        server: asid,
        namespaces: [pending.caller, asid]
            .map(|owner| ipc.caps.get(&owner).and_then(|caps| caps.address_space)),
    }))
}

fn resolve_namespace_token(
    ipc: &IpcRegistry,
    asid: AddressSpaceId,
    token: ReplyTokenId,
) -> Result<Option<Identity>, IpcError> {
    let Some(reply) = ipc.reply_tokens.get(&token) else {
        return Ok(None);
    };
    let pending = ipc.pending_calls.get(&reply.call).ok_or(IpcError::UnknownCapability)?;
    if reply.server != asid && pending.caller != asid {
        return Err(IpcError::PermissionDenied);
    }
    if reply.completing {
        return Err(IpcError::Pending);
    }
    if reply.cleanup_failed {
        return Err(IpcError::MemoryTransferFailed);
    }
    if pending.result.is_some() {
        return Err(IpcError::ReplyAlreadyUsed);
    }
    Ok(Some(Identity {
        target: CloseTarget::Namespace(asid),
        token,
        call: reply.call,
        caller: pending.caller,
        server: reply.server,
        namespaces: [pending.caller, reply.server]
            .map(|owner| ipc.caps.get(&owner).and_then(|caps| caps.address_space)),
    }))
}

impl<'a> PreparedCancellation<'a> {
    fn prepare(asid: AddressSpaceId, cap: CapabilityId) -> Result<Self, IpcError> {
        Self::prepare_with(asid, cap, None, resolve)
    }

    pub(super) fn prepare_endpoint_front(
        owner: &'a endpoint_close::PreparedEndpointClose,
    ) -> Result<Self, IpcError> {
        Self::prepare_with(
            owner.asid(),
            owner.cap(),
            Some(RootOwner::Endpoint(owner)),
            resolve_endpoint_front,
        )
    }

    pub(super) fn prepare_namespace_token(
        owner: &'a crate::memory::retirement::ClosingAddressSpace,
        token: ReplyTokenId,
    ) -> Result<Self, IpcError> {
        Self::prepare_with(
            owner.handle().id(),
            token,
            Some(RootOwner::Namespace(owner)),
            resolve_namespace_token,
        )
    }

    fn prepare_with(
        asid: AddressSpaceId,
        cap: CapabilityId,
        root_owner: Option<RootOwner<'a>>,
        resolve: fn(
            &IpcRegistry,
            AddressSpaceId,
            CapabilityId,
        ) -> Result<Option<Identity>, IpcError>,
    ) -> Result<Self, IpcError> {
        let identity = resolve(&IPC.read(), asid, cap)?.ok_or(IpcError::Pending)?;
        let mut operation = Self {
            asid,
            cap,
            identity,
            namespaces: [None, None],
            loans: Vec::new(),
            root_owner,
        };
        let prepared = (|| {
            // Lifecycle always precedes IPC. Kernel identity is permanent.
            for (index, owner) in [identity.caller, identity.server].into_iter().enumerate() {
                match operation.root_owner {
                    Some(RootOwner::Endpoint(endpoint)) => {
                        assert_eq!(identity.namespaces[1], Some(endpoint.handle()));
                        if owner == identity.server {
                            // Borrow the endpoint's pre-existing server lease.
                            continue;
                        }
                    }
                    Some(RootOwner::Namespace(namespace)) => {
                        if owner == namespace.handle().id() {
                            assert_eq!(identity.namespaces[index], Some(namespace.handle()));
                            // The Rust borrow retains the linear closing owner.
                            continue;
                        }
                        if owner == KERNEL_ASID {
                            // The kernel table entry has a captured handle too,
                            // but its permanent root must never obtain a user
                            // operation lease (which correctly rejects it).
                            continue;
                        }
                        if let Some(handle) = identity.namespaces[index] {
                            operation.namespaces[index] = Some(
                                namespace.retain_peer(handle).map_err(|error| match error {
                                    crate::memory::operation::OperationError::Closing
                                    | crate::memory::operation::OperationError::AddressSpaceMissing
                                    | crate::memory::operation::OperationError::StaleHandle => IpcError::Pending,
                                    _ => IpcError::ResourceLimit,
                                })?,
                            );
                            continue;
                        }
                        // Synthetic boot namespaces can carry scalar calls.
                        // Loans require captured roots, checked below under IPC.
                        continue;
                    }
                    None => {}
                }
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
                if borrow.owner != identity.caller
                    || borrow.borrower != identity.server
                    || [identity.caller, identity.server]
                        .into_iter()
                        .zip(identity.namespaces)
                        .any(|(owner, handle)| owner != KERNEL_ASID && handle.is_none())
                {
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

    pub(super) fn finish_with(
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
        if let CloseTarget::Endpoint(endpoint) = identity.target {
            let endpoint = &ipc.endpoints[&endpoint];
            assert!(endpoint.closing && !endpoint.closed);
            assert_eq!(endpoint.queue.front().unwrap().reply, Some(identity.token));
        }
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
            let endpoint_close = matches!(identity.target, CloseTarget::Endpoint(_))
                || matches!(identity.target, CloseTarget::Namespace(owner) if owner == identity.server);
            Some((
                message,
                Delivery {
                    observers: if endpoint_close {
                        WaitNotifications::empty()
                    } else {
                        endpoint.readiness_observers.drain()
                    },
                    cq_wake: if endpoint_close {
                        None
                    } else {
                        endpoint.notify_cq.map(|cq| (endpoint.owner, cq))
                    },
                },
            ))
        });
        let was_queued = queued.is_some();
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
            CloseTarget::Call | CloseTarget::Namespace(_) if identity.caller == self.asid => {
                let cap = if identity.target == CloseTarget::Call {
                    self.cap
                } else {
                    ipc.caps[&self.asid]
                        .caps
                        .iter()
                        .find_map(|(&id, cap)| {
                            (cap.payload
                                == Capability::PendingCall {
                                    call: identity.call,
                                })
                            .then_some(id)
                        })
                        .expect("retiring call cap missing")
                };
                ipc.remove_cap(self.asid, cap).expect("claimed call cap missing");
                ipc.pending_calls
                    .remove(&identity.call)
                    .expect("claimed pending call missing")
                    .observers
                    .close()
            }
            CloseTarget::Reply | CloseTarget::Endpoint(_) | CloseTarget::Namespace(_) => {
                if identity.target == CloseTarget::Reply {
                    assert_eq!(reply_cap, Some(self.cap));
                } else if matches!(identity.target, CloseTarget::Endpoint(_)) {
                    assert!(reply_cap.is_none(), "queued endpoint call acquired reply authority");
                }
                let call = ipc
                    .pending_calls
                    .get_mut(&identity.call)
                    .expect("claimed pending call missing");
                assert!(call.result.is_none());
                call.result = Some(ReplyValue {
                    result: if matches!(identity.target, CloseTarget::Endpoint(_))
                        || (matches!(identity.target, CloseTarget::Namespace(_)) && was_queued)
                    {
                        REPLY_ENDPOINT_CLOSED
                    } else {
                        REPLY_CANCELLED
                    },
                    cap: None,
                    memory: None,
                });
                call.observers.close()
            }
            CloseTarget::Call => unreachable!("call close must own the caller"),
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
