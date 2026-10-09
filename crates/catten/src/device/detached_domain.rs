//! Detached ownership, while the original admitted registry slot stays
//! empty and its requester remains fenced. This is not a hardware completion
//! proof: each backend must finish its own maintenance before physical release.
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

/// One containing owner for a detached domain and the actual, exclusively
/// moved command engine. The engine's backing belongs to the installed unit
/// for its whole lifetime; no queue/tail/epoch is reconstructed from a snapshot.
#[must_use]
pub(super) struct Maintenance<D, C> {
    pub(super) domain: DetachedDomain<D>,
    pub(super) commands: DetachedDomain<C>,
}

impl<D, C> Maintenance<D, C> {
    pub(super) fn new(domain: D, commands: C) -> Self {
        Self {
            domain: DetachedDomain::new(domain),
            commands: DetachedDomain::new(commands),
        }
    }
}
