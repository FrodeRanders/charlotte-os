//! Owning zeroed translation-frame preparation. This is publication/lifetime
//! protection and physical progress policy, not a translation-table quota.

use super::{
    PAddr,
    PreparingUserFrame,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableScope {
    PrivateUser,
    SharedKernel,
}

fn allowed(scope: TableScope, free: u64, usable: u64) -> bool {
    scope == TableScope::SharedKernel
        || charlotte_lifecycle::resources::frames_available(free, usable, 1)
}

#[must_use]
pub(crate) struct PreparingTable(PreparingUserFrame);

impl PreparingTable {
    pub(crate) fn allocate(scope: TableScope) -> Option<Self> {
        let frame =
            PreparingUserFrame::allocate_with_policy(|free, usable| allowed(scope, free, usable))?;
        let preparation = Self(frame);
        preparation.0.zero();
        Some(preparation)
    }

    pub(crate) fn frame(&self) -> PAddr {
        self.0.frame()
    }

    /// Final architecture boundary: all fallible preparation precedes this
    /// consuming call. Adopt into a parent link or owning root exactly once.
    /// Disarm before invocation so interrupted publication cannot recycle a
    /// potentially hardware-reachable table. No retry/recovery bypass exists.
    pub(crate) fn publish<T>(self, publish: impl FnOnce(PAddr) -> T) -> T {
        let frame = self.frame();
        self.0.quarantine();
        publish(frame)
    }

    pub(crate) fn test_policy() {
        assert!(!allowed(TableScope::PrivateUser, 10, 80));
        assert!(allowed(TableScope::PrivateUser, 11, 80));
        assert!(!allowed(TableScope::PrivateUser, 0, 80));
        assert!(allowed(TableScope::SharedKernel, 10, 80));
        // Shared-kernel policy admits a request, but the real allocator still
        // rejects exhaustion. This does not manufacture free frames.
        assert!(allowed(TableScope::SharedKernel, 0, 80));
    }
}

pub(crate) mod tests;
