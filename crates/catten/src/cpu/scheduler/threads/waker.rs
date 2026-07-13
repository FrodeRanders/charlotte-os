use alloc::sync::Arc;

use crate::{
    cpu::scheduler::{
        system_scheduler::SYSTEM_SCHEDULER,
        threads::ThreadId,
    },
    logln,
};

#[derive(Debug, PartialEq, PartialOrd, Eq, Ord)]
pub struct Waker(ThreadId);

impl Waker {
    pub fn new(tid: ThreadId) -> Self {
        Self(tid)
    }
}

impl crate::klib::observer::Observer for Waker {
    fn notify(self: Arc<Self>) {
        logln!("Waking thread with ID {}.", (self.0));
        SYSTEM_SCHEDULER
            .write()
            .submit_ready_thread(self.0)
            .expect("Error submitting ready thread to system scheduler");
    }
}
