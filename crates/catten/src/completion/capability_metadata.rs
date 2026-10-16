//! Admitted capability payload metadata carries detached authority through
//! explicit disposal. The charged completion stays alive until both nodes free.

use super::*;

pub(super) struct CapabilityRecord {
    completion: CompletionRef,
    authority: Option<crate::capability::RetiredRecord>,
}

impl CapabilityRecord {
    pub(super) fn new(completion: CompletionRef) -> Self {
        Self {
            completion,
            authority: None,
        }
    }

    pub(super) fn revoke(&mut self, asid: AddressSpaceId, cap: CompletionCap) {
        assert!(self.authority.is_none(), "completion authority detached twice");
        self.authority = Some(
            crate::capability::detach(asid, cap, crate::capability::ObjectKind::Completion)
                .expect("completion payload capability was absent from unified table"),
        );
    }
}

impl Deref for CapabilityRecord {
    type Target = CompletionRef;

    fn deref(&self) -> &CompletionRef {
        &self.completion
    }
}

/// Drop retains the complete payload node and its detached authority in the
/// shared RetiredEntry; no sibling fields can fall through into cleanup.
#[must_use = "finish both completion metadata owners after local guards"]
pub(super) struct RetiredCapability(RetiredEntry<(CompletionCap, CapabilityRecord)>);

impl RetiredCapability {
    pub(super) fn completion(&self) -> &CompletionRef {
        &self.0.value().1.completion
    }

    pub(super) fn finish(mut self) {
        let completion = self.completion().clone();
        let authority =
            self.0.value_mut().1.authority.take().expect("unrevoked completion retirement");
        // Keep the original charged Arc through actual release of both nodes.
        // Final strong/weak backing accounting remains ChargedAllocator's job.
        self.0.release();
        authority.release();
        drop(completion);
    }

    pub(super) fn from_node(node: RetiredEntry<(CompletionCap, CapabilityRecord)>) -> Self {
        Self(node)
    }
}

/// Called under COMPLETIONS after the caller's exact-object validation. Every
/// fallible lookup precedes mutation; no allocation/free/refund occurs here.
pub(super) fn retire(
    entries: &mut AsCompletions,
    asid: AddressSpaceId,
    cap: CompletionCap,
) -> Result<RetiredCapability, CapError> {
    let record = entries.table.get_mut(&cap).ok_or(CapError::UnknownCap)?;
    let operation = record.operation_id();
    record.revoke(asid, cap);
    if let Some(queue) = entries.cqs.get_mut(&DEFAULT_CQ) {
        queue.backlog.retain(|entry| {
            !matches!(&entry.owner, BacklogOwner::Capability)
                || entry.operation != operation
                || entry.cookie != cap
        });
    }
    let node = entries.table.take(&cap).unwrap();
    entries.live = entries.live.checked_sub(1).expect("retired completion slot missing");
    Ok(RetiredCapability(node))
}
