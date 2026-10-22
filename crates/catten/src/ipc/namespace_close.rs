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

/// Selected under IPC before cancellation preparation. A later rejection may
/// reflect a changed episode; this observation grants no claim/cleanup proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingToken {
    pub token: u64,
    pub call: u64,
    pub caller: Option<crate::memory::AddressSpaceHandle>,
    pub server: Option<crate::memory::AddressSpaceHandle>,
    pub epoch: u64,
    pub completing: bool,
    pub cleanup_failed: bool,
    pub loans: usize,
}

pub(crate) enum NamespaceProgress {
    Pending(PendingToken),
    Complete,
}

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
) -> Result<NamespaceProgress, IpcError> {
    let asid = owner.handle().id();
    loop {
        let next = {
            let ipc = IPC.read();
            ipc.reply_tokens.iter().find_map(|(&id, token)| {
                (token.server == asid
                    || ipc.pending_calls.get(&token.call).is_some_and(|call| call.caller == asid))
                .then(|| PendingToken {
                    token: id,
                    call: token.call,
                    caller: ipc.pending_calls.get(&token.call).and_then(|call| {
                        ipc.caps.get(&call.caller).and_then(|caps| caps.address_space)
                    }),
                    server: ipc.caps.get(&token.server).and_then(|caps| caps.address_space),
                    epoch: token.claim_epoch,
                    completing: token.completing,
                    cleanup_failed: token.cleanup_failed,
                    loans: token.borrows.len(),
                })
            })
        };
        let Some(token) = next else {
            break;
        };
        match cancellation::PreparedCancellation::prepare_namespace_token(owner, token.token) {
            Ok(call) => call.finish_with(&mut finish)?,
            Err(IpcError::Pending) => return Ok(NamespaceProgress::Pending(token)),
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
    // attachment authority is retired under IPC; admitted storage carries its
    // backing past unlock. Undelivered owners cannot acquire mappings or pins.
    drain_namespace_caps(asid)?;
    Ok(NamespaceProgress::Complete)
}
