//! Complete logical cancellation handback; no reference/backing release under
//! the cancelling queue transaction. This supplies no producer/physical proof.

use core::mem::ManuallyDrop;

use super::{
    RetiredEvents,
    TimerEventCancelHandle,
};

#[must_use = "finish cancellation after enclosing queue/registry guards leave"]
pub(crate) struct CancelledTimer(ManuallyDrop<Resources>);

struct Resources {
    handle: TimerEventCancelHandle,
    retired: RetiredEvents,
    removed: bool,
}

impl CancelledTimer {
    pub(super) fn new(handle: TimerEventCancelHandle) -> Self {
        Self(ManuallyDrop::new(Resources {
            handle,
            retired: RetiredEvents::default(),
            removed: false,
        }))
    }

    pub(super) fn handle(&self) -> &TimerEventCancelHandle {
        &self.0.handle
    }

    pub(super) fn record(&mut self, removed: bool, retired: RetiredEvents) {
        self.0.removed = removed;
        self.0.retired = retired;
    }

    pub(crate) fn removed(&self) -> bool {
        self.0.removed
    }

    pub(crate) fn finish(self) -> bool {
        self.finish_with(|| {})
    }

    fn finish_with(self, mut before_dispose: impl FnMut()) -> bool {
        let Resources {
            handle,
            retired,
            removed,
        } = ManuallyDrop::into_inner(self.0);
        retired.finish_with(|_| before_dispose());
        before_dispose();
        handle.release();
        removed
    }
}

mod tests;

pub(super) fn test_disposal() {
    tests::run();
}
