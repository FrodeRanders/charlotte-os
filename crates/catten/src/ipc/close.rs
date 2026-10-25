//! IPC capability close dispatch and serialized authority/namespace draining.
//! Exact pending owners, claims and original cleanup ordering remain in their
//! existing cancellation/reply/endpoint families; no retry custody is added.
use super::*;

/// Internal execution policy, not evidence of completion or caller authority.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CloseMode {
    /// Visible capability admission; loan cleanup prepares its exact root owners
    /// outside IPC and may wait through the existing captured claim interest.
    Owned,
    /// Root/boot-fixture cleanup after claim/lease drain. Hidden grants may be
    /// consumed, but this adapter never admits fresh live roots beneath IPC.
    Serialized,
}

pub fn close_cap(asid: AddressSpaceId, cap: CapabilityId) -> Result<(), IpcError> {
    let mut owner = claim_wait::Wait::new();
    let result = close_cap_in_mode(asid, cap, CloseMode::Owned, || owner.wait(asid, cap));
    owner.finish(result)
}

pub(super) fn close_cap_with_wait(
    asid: AddressSpaceId,
    cap: CapabilityId,
    mut wait: impl FnMut(),
) -> Result<(), IpcError> {
    close_cap_in_mode(asid, cap, CloseMode::Owned, || {
        wait();
        Ok(())
    })
}

/// Root/fixture cleanup owns namespace retirement and has drained claims.
/// It may consume hidden connection grants but must not acquire new live leases.
pub(super) fn close_cap_serialized(
    asid: AddressSpaceId,
    cap: CapabilityId,
) -> Result<(), IpcError> {
    close_cap_in_mode(asid, cap, CloseMode::Serialized, || Err(IpcError::Pending))
}

fn close_cap_in_mode(
    asid: AddressSpaceId,
    cap: CapabilityId,
    mode: CloseMode,
    wait: impl FnMut() -> Result<(), IpcError>,
) -> Result<(), IpcError> {
    close_cap_in_mode_with_revoker(asid, cap, mode, wait, revoke_memory_borrow)
}

// The boot fixture supplies an actual prepared-loan failure. Production uses
// only the serialized revocation adapter, never arbitrary callback work.
pub(super) fn close_cap_in_mode_with_revoker(
    asid: AddressSpaceId,
    cap: CapabilityId,
    mode: CloseMode,
    wait: impl FnMut() -> Result<(), IpcError>,
    revoke: impl FnMut(MemoryBorrow) -> Result<(), crate::memory::object::MemoryObjectError>,
) -> Result<(), IpcError> {
    close_cap_with_cleanup(asid, cap, mode, wait, revoke, attachments::release_one)
}

pub(super) fn close_cap_with_cleanup(
    asid: AddressSpaceId,
    cap: CapabilityId,
    mode: CloseMode,
    mut wait: impl FnMut() -> Result<(), IpcError>,
    mut revoke: impl FnMut(MemoryBorrow) -> Result<(), crate::memory::object::MemoryObjectError>,
    mut release: impl FnMut(crate::memory::object::RetiredMemory),
) -> Result<(), IpcError> {
    // Keep the caller's borrow alive until a claimed reply has finished its
    // detached cleanup. Never wait with IPC serialization held. Domain cleanup
    // reaches here only after the reply's exact-generation root leases drain.
    let mut retired_replies = RetiredReplies::new();
    let mut retired_metadata = RetiredMetadata::new();
    let mut ipc = loop {
        let ipc = IPC.write();
        let payload = if mode == CloseMode::Owned {
            ipc.cap(asid, cap)?
        } else {
            ipc.cap_for_cleanup(asid, cap)?
        };
        let busy = claim_wait::is_busy(&ipc, asid, cap, payload);
        if !busy {
            if mode == CloseMode::Owned && cancellation::has_loans(&ipc, asid, cap)? {
                let endpoint = matches!(ipc.cap(asid, cap)?, Capability::Endpoint { .. });
                drop(ipc);
                // Revalidate identity and claim after admitting both roots.
                // Another close/reply can win this preparation interval.
                let result = if endpoint {
                    endpoint_close::close(asid, cap)
                } else {
                    cancellation::close(asid, cap)
                };
                match result {
                    Err(IpcError::Pending) => {
                        wait()?;
                        continue;
                    }
                    result => return result,
                }
            }
            break ipc;
        }
        drop(ipc);
        wait()?;
    };
    // Preserve the close capability, queue and pending record until every loan
    // has confirmed cleanup. Partial success is recorded in place; a failed
    // token stays fenced and cannot be delivered or replied to again.
    prepare_serialized_close(&mut ipc, asid, cap, &mut revoke)?;
    let mut observers = WaitNotifications::empty();
    let mut cq_wake = None;
    let mut close_watches = None;
    let mut retired_queue = budget::AdmittedQueue::default();
    let mut retired_message = None;
    let mut retired_result = None;
    let mut retired_call = None;
    match ipc.remove_cap(asid, cap, &mut retired_metadata)? {
        Capability::Endpoint {
            endpoint,
            ..
        } => {
            let mut queued = if let Some(endpoint) = ipc.endpoints.get_mut(&endpoint) {
                if endpoint.owner != asid {
                    budget::AdmittedQueue::default()
                } else {
                    endpoint.closed = true;
                    observers.append(endpoint.readiness_observers.close());
                    close_watches = Some(endpoint.close_observers.close());
                    // A CQ-bound endpoint reports its closure as a readiness
                    // wake so a reactor blocked on one CQ wait observes it.
                    cq_wake = endpoint.notify_cq.map(|cq| (endpoint.owner, cq));
                    core::mem::take(&mut endpoint.queue)
                }
            } else {
                budget::AdmittedQueue::default()
            };
            for message in queued.iter_mut() {
                if let Some(token) = message.reply {
                    consume_reply_token(
                        &mut ipc,
                        token,
                        REPLY_ENDPOINT_CLOSED,
                        &mut observers,
                        &mut retired_replies,
                    )
                    .expect("preflighted endpoint loan cleanup changed under IPC");
                }
                message.memory.retire(asid);
                if let Some(connection_cap) = message.connection {
                    let _ = ipc.remove_cap(asid, connection_cap, &mut retired_metadata);
                }
            }
            retired_queue = queued;
            // Retire the registry entry once no capability can name it, so
            // endpoint create/close cycles cannot grow the registry forever.
            if let Some(node) = ipc.detach_endpoint(endpoint) {
                retired_metadata.push(node);
            }
        }
        Capability::PendingCall {
            call,
        } => {
            if let Some(mut node) = ipc.pending_calls.take(&call) {
                let pending = &mut node.value_mut().1;
                // A thread parked in wait_reply holds its only waker here.
                observers.append(pending.observers.close());
                if let Some(reply) = pending.result {
                    // Revoke undelivered results only. Once the caller has
                    // observed the reply, the returned capabilities are its
                    // property and survive the pending-call close.
                    if !pending.observed {
                        if let Some(returned_cap) = reply.cap {
                            let _ = ipc.remove_cap(asid, returned_cap, &mut retired_metadata);
                        }
                        if let Some(memory_cap) = reply.memory {
                            retired_result = attachments::retire_one(asid, memory_cap);
                        }
                    }
                } else {
                    retired_message = cancel_queued_call(
                        &mut ipc,
                        call,
                        &mut observers,
                        &mut cq_wake,
                        &mut retired_replies,
                        &mut retired_metadata,
                    );
                }
                retired_call = Some(node);
            }
        }
        Capability::ReplyToken {
            token,
        } => {
            consume_reply_token(
                &mut ipc,
                token,
                REPLY_CANCELLED,
                &mut observers,
                &mut retired_replies,
            )
            .expect("preflighted reply loan cleanup changed under IPC");
        }
        Capability::Connection {
            endpoint,
            ..
        } => {
            // The last capability naming a closed endpoint retires it.
            if let Some(node) = ipc.detach_endpoint(endpoint) {
                retired_metadata.push(node);
            }
        }
    }
    drop(ipc);
    retired_replies.release();
    retired_metadata.release();
    if let Some(node) = retired_call {
        node.release();
    }
    if let Some(owner) = retired_result {
        release(owner);
    }
    if let Some(memory) = retired_message {
        memory.release_with(&mut release);
    }
    for message in retired_queue.drain(..) {
        message.memory.release_with(&mut release);
    }
    drop(retired_queue);
    if let Some(watches) = close_watches {
        watches.notify();
    }
    if let Some((owner, cq)) = cq_wake {
        crate::completion::wake(owner, cq);
    }
    signal_observers(observers);
    Ok(())
}

/// Raw kernel boot-fixture cleanup. Fixtures also exercise synthetic namespaces
/// that have no ClosingAddressSpace. Production root close exclusively uses
/// namespace_close with a borrowed closing owner; do not use this serialized
/// loan adapter for published runtime domains.
pub(crate) fn close_address_space_fixture(asid: AddressSpaceId) -> Result<(), IpcError> {
    {
        let mut ipc = IPC.write();
        assert!(
            !ipc.reply_tokens.values().any(|token| token.completing
                && (token.server == asid
                    || ipc.pending_calls.get(&token.call).is_some_and(|call| call.caller == asid))),
            "IPC namespace cleanup before completion leases drained"
        );
        if let Some(namespace) = ipc.caps.get(&asid) {
            namespace.record_budget.retire();
        }
        // Preflight every token involving this namespace, including delivered
        // replies and foreign callers. Do not remove any capability first:
        // root cleanup must not lose the receipt for an uncertain loan.
        let mut cursor = 0;
        loop {
            let next = ipc.reply_tokens.iter().find_map(|(&id, token)| {
                (id > cursor
                    && (token.server == asid
                        || ipc
                            .pending_calls
                            .get(&token.call)
                            .is_some_and(|call| call.caller == asid)))
                .then_some(id)
            });
            let Some(token) = next else {
                break;
            };
            revoke_serialized_token(&mut ipc, token, &mut revoke_memory_borrow)?;
            cursor = token;
        }
    }
    drain_namespace_caps(asid)
}

pub(super) fn drain_namespace_caps(asid: AddressSpaceId) -> Result<(), IpcError> {
    // Reuse admitted registry storage as the work list to avoid allocating
    // a teardown snapshot. Closing one call may also remove a reply cap.
    loop {
        let next = IPC
            .read()
            .caps
            .get(&asid)
            .and_then(|caps| caps.caps.first_key_value().map(|(id, _)| *id));
        let Some(cap) = next else {
            break;
        };
        close_cap_serialized(asid, cap)?;
    }
    // Detach the complete namespace. Its fallback retains the node and both
    // budget accounts; explicit disposal runs after the IPC guard leaves.
    let retired = IPC.write().caps.take(&asid);
    if let Some(retired) = retired {
        assert!(retired.value().1.caps.first_key_value().is_none());
        retired.release();
    }
    let mut retired = RetiredMetadata::new();
    {
        let mut ipc = IPC.write();
        loop {
            let next = ipc.endpoints.iter().find_map(|(&id, endpoint)| {
                (endpoint.closed && !endpoint_referenced(&ipc, id)).then_some(id)
            });
            let Some(id) = next else {
                break;
            };
            retired.push(ipc.detach_endpoint(id).unwrap());
        }
    }
    retired.release();
    Ok(())
}

fn revoke_serialized_token(
    ipc: &mut IpcRegistry,
    token: ReplyTokenId,
    revoke: &mut impl FnMut(MemoryBorrow) -> Result<(), crate::memory::object::MemoryObjectError>,
) -> Result<(), IpcError> {
    if let Some(reply) = ipc.reply_tokens.get(&token) {
        if reply.completing {
            return Err(IpcError::Pending);
        }
        if reply.cleanup_failed {
            return Err(IpcError::MemoryTransferFailed);
        }
    }
    while let Some(borrow) =
        ipc.reply_tokens.get(&token).and_then(|token| token.borrows.last().copied())
    {
        if revoke(borrow).is_err() {
            ipc.reply_tokens.get_mut(&token).unwrap().cleanup_failed = true;
            crate::logln!(
                "[IPC] bulk cancellation retained token {} after uncertain loan cleanup",
                token
            );
            return Err(IpcError::MemoryTransferFailed);
        }
        assert_eq!(ipc.reply_tokens.get_mut(&token).unwrap().borrows.pop(), Some(borrow));
    }
    Ok(())
}

fn prepare_serialized_close(
    ipc: &mut IpcRegistry,
    asid: AddressSpaceId,
    cap: CapabilityId,
    revoke: &mut impl FnMut(MemoryBorrow) -> Result<(), crate::memory::object::MemoryObjectError>,
) -> Result<(), IpcError> {
    match ipc.cap_for_cleanup(asid, cap)? {
        Capability::Endpoint {
            endpoint,
            ..
        } => {
            let mut index = 0;
            loop {
                let message = ipc
                    .endpoints
                    .get(&endpoint)
                    .filter(|endpoint| endpoint.owner == asid)
                    .and_then(|endpoint| endpoint.queue.get(index));
                let Some(message) = message else {
                    break;
                };
                if let Some(token) = message.reply {
                    revoke_serialized_token(ipc, token, revoke)?;
                }
                index += 1;
            }
        }
        Capability::PendingCall {
            call,
        } => {
            // Keep the token in the registry until its loans are confirmed.
            let mut cursor = 0;
            loop {
                let next = ipc
                    .reply_tokens
                    .iter()
                    .find_map(|(&id, token)| (id > cursor && token.call == call).then_some(id));
                let Some(token) = next else {
                    break;
                };
                revoke_serialized_token(ipc, token, revoke)?;
                cursor = token;
            }
        }
        Capability::ReplyToken {
            token,
        } => revoke_serialized_token(ipc, token, revoke)?,
        Capability::Connection {
            ..
        } => {}
    }
    Ok(())
}

pub(super) fn consume_reply_token(
    ipc: &mut IpcRegistry,
    token: ReplyTokenId,
    result: i64,
    observers: &mut WaitNotifications,
    retired: &mut RetiredReplies,
) -> Result<(), IpcError> {
    revoke_serialized_token(ipc, token, &mut revoke_memory_borrow)?;
    if let Some(node) = ipc.reply_tokens.take(&token) {
        let token = &node.value().1;
        assert!(
            !token.completing && token.borrows.is_empty(),
            "consuming an unfinished completion"
        );
        if let Some(call) = ipc.pending_calls.get_mut(&token.call) {
            call.result = Some(ReplyValue {
                result,
                cap: None,
                memory: None,
            });
            observers.append(call.observers.close());
        }
        retired.push(node);
    }
    Ok(())
}

fn cancel_queued_call(
    ipc: &mut IpcRegistry,
    call: PendingCallId,
    observers: &mut WaitNotifications,
    cq_wake: &mut Option<(AddressSpaceId, crate::completion::CqId)>,
    retired: &mut RetiredReplies,
    retired_metadata: &mut RetiredMetadata,
) -> Option<MemoryAttachments> {
    let mut memory = None;
    loop {
        let next = ipc
            .reply_tokens
            .iter()
            .find_map(|(&id, token)| (token.call == call).then_some((id, token.server)));
        let Some((token, server)) = next else {
            break;
        };
        let node = ipc.reply_tokens.take(&token).unwrap();
        let payload = &node.value().1;
        assert!(
            !payload.completing && !payload.cleanup_failed && payload.borrows.is_empty(),
            "cancelling an unfinished reply"
        );
        if let Some(retired) = cancel_queued_message_with_token(
            ipc,
            server,
            token,
            observers,
            cq_wake,
            retired_metadata,
        ) {
            assert!(memory.is_none(), "one call acquired multiple queued messages");
            memory = Some(retired);
        }
        ipc.remove_matching_caps(
            server,
            Capability::ReplyToken {
                token,
            },
            retired_metadata,
        );
        retired.push(node);
    }
    memory
}

fn cancel_queued_message_with_token(
    ipc: &mut IpcRegistry,
    server: AddressSpaceId,
    token: ReplyTokenId,
    observers: &mut WaitNotifications,
    cq_wake: &mut Option<(AddressSpaceId, crate::completion::CqId)>,
    retired_metadata: &mut RetiredMetadata,
) -> Option<MemoryAttachments> {
    let selected = ipc.endpoints.iter().find_map(|(&id, endpoint)| {
        if endpoint.owner != server {
            return None;
        }
        endpoint
            .queue
            .iter()
            .position(|message| message.reply == Some(token))
            .map(|index| (id, index))
    });
    let (id, index) = selected?;
    let endpoint = ipc.endpoints.get_mut(&id).unwrap();
    observers.append(endpoint.readiness_observers.drain());
    *cq_wake = endpoint.notify_cq.map(|cq| (endpoint.owner, cq));
    let mut message = endpoint.queue.remove(index)?;
    message.memory.retire(server);
    if let Some(connection_cap) = message.connection {
        ipc.remove_cap(server, connection_cap, retired_metadata)
            .expect("queued connection disappeared during cancellation");
    }
    Some(message.memory)
}
