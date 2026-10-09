//! Post-maintenance ownership, while the original admitted registry slot stays
//! empty and its requester remains fenced. This is not a hardware completion
//! proof: each backend must finish its own typed maintenance before extraction.
use core::mem::ManuallyDrop;

#[must_use]
pub(super) struct DetachedDomain<T> {
    value: ManuallyDrop<T>,
}

impl<T> DetachedDomain<T> {
    pub(super) fn new(value: T) -> Self {
        Self {
            value: ManuallyDrop::new(value),
        }
    }

    pub(super) fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// Explicitly restore the exact admitted slot on ordinary rejection, or
    /// consume collections outside serialization after confirmed completion.
    pub(super) fn into_inner(self) -> T {
        ManuallyDrop::into_inner(self.value)
    }
}

// No Drop: ManuallyDrop retains the whole domain, including table ledger,
// mapping/quarantine storage and pins. Abandonment leaves the existing empty
// slot and requester fence in place; no registry, allocator or logger is entered.
