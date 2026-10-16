//! Bounded captured due-prefix ownership through post-queue delivery.
//! Callback/disposal run on the original LP with the caller's IRQ mask intact.

use core::{
    marker::PhantomData,
    mem::ManuallyDrop,
};

use super::*;

#[must_use = "deliver after releasing the queue borrow on the captured LP"]
// A local claim cannot be sent/shared as work for another LP. Runtime context
// checks additionally reject migration of the executing callback continuation.
pub(crate) struct PendingDelivery(ManuallyDrop<Resources>, PhantomData<*mut ()>);

struct Resources {
    owner_lp: crate::cpu::isa::lp::LpId,
    ready: RetiredEvents,
    retired: RetiredEvents,
}

impl PendingDelivery {
    pub(super) fn new(ready: RetiredEvents, retired: RetiredEvents) -> Self {
        Self(
            ManuallyDrop::new(Resources {
                owner_lp: crate::cpu::isa::lp::ops::get_lp_id(),
                ready,
                retired,
            }),
            PhantomData,
        )
    }

    pub(crate) fn deliver(self) {
        self.deliver_with(|| {
            TIMER_QUEUES
                .try_get_mut()
                .expect("timer callback retained queue borrow")
                .complete_delivery();
        });
    }

    /// Serialized comparator fixtures finish their original private queue.
    /// Production always completes this LP's existing TIMER_QUEUES claim.
    pub(super) fn deliver_with(mut self, complete: impl FnOnce()) {
        self.check_context();
        core::mem::take(&mut self.0.retired).finish();
        while let Some(event) = self.0.ready.pop_front() {
            self.check_context();
            // Recheck retained cancellation just before selecting callback and
            // waiter delivery. Logical queue absence is no terminal result.
            event.front().unwrap().signal();
            self.check_context();
            event.finish();
        }
        self.check_context();
        complete();
        drop(ManuallyDrop::into_inner(self.0));
    }

    fn check_context(&self) {
        assert_eq!(
            crate::cpu::isa::lp::ops::get_lp_id(),
            self.0.owner_lp,
            "timer delivery migrated away from its owning LP"
        );
        assert!(
            !crate::cpu::isa::lp::ops::get_int_state(),
            "timer delivery lost its enclosing IRQ mask"
        );
    }
}

mod tests;

pub(super) fn test_delivery() {
    tests::run();
}
