//! Boot-lifetime operation identities. Exhaustion is permanent; IDs are never
//! refunded, including after rejected preparation or namespace replacement.

use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

// u64::MAX is the completion-submission failure sentinel at the syscall ABI.
pub(crate) const LAST_OPERATION_ID: u64 = u64::MAX - 1;

pub(crate) struct OperationIds {
    last_issued: AtomicU64,
}

impl OperationIds {
    /// Production starts at zero. Boundary fixtures use independent counters;
    /// the live global counter has no reset or replacement interface.
    pub(crate) const fn new(last_issued: u64) -> Self {
        Self {
            last_issued: AtomicU64::new(last_issued),
        }
    }

    pub(crate) fn allocate(&self) -> Option<u64> {
        // Only uniqueness is synchronized here. Registry/source publication
        // supplies the ordering for the associated payload and authority.
        self.last_issued
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
                last.checked_add(1).filter(|next| *next <= LAST_OPERATION_ID)
            })
            .ok()
            .map(|last| last + 1)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        sync::Barrier,
        thread,
    };

    use super::*;

    #[test]
    fn starts_nonzero_and_does_not_refund_discarded_ids() {
        let ids = OperationIds::new(0);
        assert_eq!(ids.allocate(), Some(1));
        // Discarding an unpublished identity does not make it available again.
        assert_eq!(ids.allocate(), Some(2));
    }

    #[test]
    fn issues_last_id_once_and_stays_exhausted() {
        let ids = OperationIds::new(LAST_OPERATION_ID - 2);
        assert_eq!(ids.allocate(), Some(LAST_OPERATION_ID - 1));
        assert_eq!(ids.allocate(), Some(LAST_OPERATION_ID));
        for _ in 0..128 {
            assert_eq!(ids.allocate(), None);
        }
        assert_eq!(ids.last_issued.load(Ordering::Relaxed), LAST_OPERATION_ID);
    }

    #[test]
    fn sentinel_state_cannot_wrap_to_zero() {
        let ids = OperationIds::new(u64::MAX);
        assert_eq!(ids.allocate(), None);
        assert_eq!(ids.last_issued.load(Ordering::Relaxed), u64::MAX);
    }

    fn concurrent_ids(last: u64, per_thread: usize) -> Vec<u64> {
        let ids = OperationIds::new(last);
        let start = Barrier::new(8);
        thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        (0..per_thread).filter_map(|_| ids.allocate()).collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|handle| handle.join().unwrap()).collect()
        })
    }

    #[test]
    fn concurrent_allocations_are_unique() {
        let issued = concurrent_ids(0, 1_024);
        assert_eq!(issued.len(), 8_192);
        assert_eq!(issued.into_iter().collect::<BTreeSet<_>>(), (1..=8_192).collect());
    }

    #[test]
    fn concurrent_exhaustion_never_wraps_or_duplicates() {
        let issued = concurrent_ids(LAST_OPERATION_ID - 31, 64);
        assert_eq!(issued.len(), 31);
        assert_eq!(
            issued.into_iter().collect::<BTreeSet<_>>(),
            (LAST_OPERATION_ID - 30..=LAST_OPERATION_ID).collect()
        );
    }
}
