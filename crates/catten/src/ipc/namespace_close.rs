//! Whole-domain loan cleanup borrows the closing namespace's exact root.
//! Peer leases and bounded receipts survive outside IPC/lifecycle. Claims and
//! admission fences remain on abandonment; no teardown snapshot is allocated.

use super::*;
use crate::memory::{
    object::{
        LoanRevocation,
        MemoryObjectError,
    },
    retirement::ClosingAddressSpace,
};

pub(crate) fn begin(owner: &ClosingAddressSpace) -> Result<(), IpcError> {
    let handle = owner.handle();
    let mut ipc = IPC.write();
    if let Some(namespace) = ipc.caps.get(&handle.id()) {
        if namespace.address_space != Some(handle) {
            return Err(IpcError::PermissionDenied);
        }
        namespace.record_budget.retire();
    }
    // Existing explicit endpoint owners must have drained before this phase.
    // Fence even empty queues: foreign callers cannot create new work while
    // this namespace is unlocked to revoke earlier loans.
    for endpoint in ipc.endpoints.values_mut().filter(|endpoint| endpoint.owner == handle.id()) {
        assert!(!endpoint.closing, "namespace cleanup before endpoint owner drained");
        if !endpoint.closed {
            endpoint.closing = true;
        }
    }
    Ok(())
}

/// Pending returns to the caller with its closing owner intact, without
/// spinning or sleeping under a registry/lifecycle guard.
pub(crate) fn close_with(
    owner: &ClosingAddressSpace,
    mut finish: impl FnMut(LoanRevocation) -> Result<(), MemoryObjectError>,
) -> Result<bool, IpcError> {
    let asid = owner.handle().id();
    loop {
        let next = {
            let ipc = IPC.read();
            ipc.reply_tokens.iter().find_map(|(&id, token)| {
                (token.server == asid
                    || ipc.pending_calls.get(&token.call).is_some_and(|call| call.caller == asid))
                .then_some(id)
            })
        };
        let Some(token) = next else {
            break;
        };
        match cancellation::PreparedCancellation::prepare_namespace_token(owner, token) {
            Ok(call) => call.finish_with(&mut finish)?,
            Err(IpcError::Pending) => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    {
        let mut ipc = IPC.write();
        // Calls into this namespace are fenced; its threads are quiescent and
        // record sponsorship is retired. No new token may appear after drain.
        assert!(!ipc.reply_tokens.values().any(|token| {
            token.server == asid
                || ipc.pending_calls.get(&token.call).is_some_and(|call| call.caller == asid)
        }));
        for endpoint in ipc.endpoints.values_mut().filter(|endpoint| endpoint.owner == asid) {
            endpoint.closed = true;
            endpoint.closing = false;
        }
    }
    // Loan-free authority cleanup uses admitted registry storage as its work
    // list. No new root lease is acquired under IPC. Existing move/copy/result
    // attachment cleanup uses the serialized memory adapter. Undelivered
    // move/copy authority cannot acquire application mappings or pins.
    drain_namespace_caps(asid)?;
    Ok(true)
}
