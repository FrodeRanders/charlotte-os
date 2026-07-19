use alloc::sync::Arc;

use crate::{
    cpu::scheduler::{
        system_scheduler::SYSTEM_SCHEDULER,
        threads::{
            ThreadGeneration,
            ThreadId,
        },
    },
    logln,
};

#[derive(Debug, PartialEq, PartialOrd, Eq, Ord)]
pub struct Waker(ThreadId, ThreadGeneration);

impl Waker {
    pub fn new(tid: ThreadId, generation: ThreadGeneration) -> Self {
        Self(tid, generation)
    }
}

impl crate::klib::observer::Observer for Waker {
    fn notify(self: Arc<Self>) {
        logln!("Waking thread with ID {}.", (self.0));
        // A notification may race thread exit and table-slot reuse. Treat a
        // generation mismatch as a stale wake, never as authority to wake the
        // new occupant of the same numeric id.
        let _ = SYSTEM_SCHEDULER.read().submit_woken_thread(self.0, self.1);
    }
}
