//! Original event ownership precedes the fallible empty-node allocation.
//! The allocator never receives payload; completed storage initializes once.

use alloc::boxed::Box;
use core::mem::{
    ManuallyDrop,
    MaybeUninit,
};

use super::{
    EventAdmission,
    PreparedEvent,
    TimerEvent,
    queue,
};

#[must_use = "complete private node initialization or explicitly cancel preparation"]
struct PreparingNode(ManuallyDrop<Resources>);

struct Resources {
    event: Option<TimerEvent>,
    admission: Option<EventAdmission>,
    storage: Option<Box<MaybeUninit<queue::Node>>>,
}

impl PreparingNode {
    fn new(event: TimerEvent) -> Self {
        assert!(event.key.is_none() && event._charge.is_some());
        let mut preparing = Self(ManuallyDrop::new(Resources {
            event: Some(event),
            admission: None,
            storage: None,
        }));
        // Capture every payload field first. This clone allocates no storage
        // and follows the same original admission through actual Box release.
        preparing.0.admission =
            Some(preparing.0.event.as_ref().unwrap()._charge.as_ref().unwrap().clone());
        preparing
    }

    fn allocate(
        &mut self,
        allocate: impl FnOnce() -> Result<Box<MaybeUninit<queue::Node>>, ()>,
    ) -> Result<(), ()> {
        assert!(self.0.storage.is_none(), "timer node storage already prepared");
        self.0.storage = Some(allocate()?);
        Ok(())
    }

    fn complete(self) -> PreparedEvent {
        assert!(self.0.storage.is_some(), "timer node initialization without storage");
        let mut resources = ManuallyDrop::into_inner(self.0);
        // No fallible work, callback or allocation remains. Box::write is the
        // one initialization of this private allocation, before queue exposure.
        let node = Box::write(
            resources.storage.take().unwrap(),
            queue::Node::new(resources.event.take().unwrap()),
        );
        let node = queue::OwnedNode::new(node, resources.admission.take().unwrap());
        PreparedEvent {
            node: ManuallyDrop::new(node),
        }
    }

    fn cancel_unpublished(self) {
        self.cancel_with(|| {});
    }

    fn cancel_with(self, mut before_dispose: impl FnMut()) {
        let Resources {
            event,
            admission,
            storage,
        } = ManuallyDrop::into_inner(self.0);
        // Empty Box backing leaves before the event, and the outside admission
        // owner outlives both. Never initialize just to run ordinary rollback.
        before_dispose();
        drop(storage);
        before_dispose();
        drop(event);
        before_dispose();
        drop(admission);
    }
}

pub(super) fn prepare(
    event: TimerEvent,
    allocate: impl FnOnce() -> Result<Box<MaybeUninit<queue::Node>>, ()>,
) -> Result<PreparedEvent, ()> {
    let mut preparing = PreparingNode::new(event);
    if let Err(error) = preparing.allocate(allocate) {
        // The allocation callback owns no event and has already returned.
        preparing.cancel_unpublished();
        return Err(error);
    }
    Ok(preparing.complete())
}

mod tests;

pub(super) fn test_preparation() {
    tests::run();
}
