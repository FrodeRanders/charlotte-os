//! Authority metadata ownership, independent of subsystem payload completion.
use core::{
    fmt,
    mem::ManuallyDrop,
};

use super::*;
use crate::klib::collections::retirement_list::RetiredEntry;

struct Storage {
    node: Option<PreparedEntry<(ObjectCapability, Entry)>>,
    charge: Option<budget::Charge>,
}
/// Contains every provisional allocation/charge across insertion rejection.
/// Inert fallback must not refund or deallocate beneath unknown outer guards.
pub(super) struct PreparingRecord(ManuallyDrop<Storage>);
impl PreparingRecord {
    pub(super) fn try_new() -> Result<Self, AllocationError> {
        let entry_irq = crate::cpu::isa::lp::ops::get_int_state();
        if record_tests::reject() {
            return Err(AllocationError::AllocationFailed);
        }
        let node = PreparedEntry::try_new().map_err(|_| AllocationError::AllocationFailed)?;
        record_tests::boundary(false, entry_irq);
        Ok(Self(ManuallyDrop::new(Storage {
            node: Some(node),
            charge: None,
        })))
    }

    pub(super) fn insert(
        &mut self,
        table: &mut AddressSpaceCapabilities,
        kind: ObjectKind,
        platform: bool,
        state: EntryState,
    ) -> Result<ObjectCapability, AllocationError> {
        assert!(self.0.node.is_some() && self.0.charge.is_none());
        self.0.charge = Some(budget::reserve(&table.budget, platform)?);
        let (cap, next) = charlotte_lifecycle::claim_generation(table.next_serial)
            .ok_or(AllocationError::IdentityExhausted)?;
        table.next_serial = next;
        table.objects.insert(
            self.0.node.take().unwrap(),
            cap,
            Entry {
                kind,
                state,
                _charge: self.0.charge.take().unwrap(),
            },
        );
        Ok(cap)
    }

    /// Ordinary completion/rejection follows capability unlock. Captured
    /// callers still have their own outer-context obligations.
    pub(super) fn finish(mut self) {
        let entry_irq = crate::cpu::isa::lp::ops::get_int_state();
        if self.0.node.is_some() || self.0.charge.is_some() {
            record_tests::boundary(true, entry_irq);
        }
        drop(self.0.node.take());
        drop(self.0.charge.take());
        record_tests::boundary_after(entry_irq);
    }

    pub(super) fn test_charge(&mut self, account: &Arc<budget::DomainBudget>) {
        self.0.charge = Some(budget::reserve(account, false).unwrap());
    }
}

impl Drop for PreparingRecord {
    fn drop(&mut self) {
        // ManuallyDrop retains every unfinished node/charge, including before
        // serial minting. Ordinary finish has already consumed both fields.
    }
}

/// Detached authority retains its original entry, class/account and node.
/// Only explicit release refunds admission; Drop quarantines the entire node.
#[must_use]
pub(crate) struct RetiredRecord(RetiredEntry<(ObjectCapability, Entry)>);
impl fmt::Debug for RetiredRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetiredRecord").finish_non_exhaustive()
    }
}
impl RetiredRecord {
    pub(super) fn new(node: RetiredEntry<(ObjectCapability, Entry)>) -> Self {
        Self(node)
    }

    pub(crate) fn release(self) {
        let entry_irq = crate::cpu::isa::lp::ops::get_int_state();
        record_tests::boundary(true, entry_irq);
        self.0.release();
        record_tests::boundary_after(entry_irq);
    }
}
