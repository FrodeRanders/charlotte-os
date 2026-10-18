//! # Deferred Work Manager
//!
//! Placeholder for work deferred from interrupt handlers to kernel threads.
//! `DeferredTask` currently has no variants. The manager holds an unbounded
//! MPMC queue, and each spawned worker attempts to take one task before exiting.
//! There is no persistent worker loop or completion reporting yet.

use concurrent_queue::ConcurrentQueue;
use spin::LazyLock;

use crate::{
    cpu::scheduler::{
        system_scheduler::SYSTEM_SCHEDULER,
        threads::Thread,
    },
    memory::KERNEL_ASID,
};

pub static DWM: LazyLock<DeferredWorkManager> = LazyLock::new(DeferredWorkManager::new);

#[derive(Debug, Clone)]
pub enum DeferredTask {}
pub struct DeferredWorkManager {
    queue: ConcurrentQueue<DeferredTask>,
}

impl DeferredWorkManager {
    pub fn new() -> Self {
        Self {
            queue: ConcurrentQueue::unbounded(),
        }
    }

    pub fn submit(&self, task: DeferredTask) {
        self.queue.push(task).unwrap();
    }

    extern "C" fn do_work() {
        if let Ok(task) = DWM.queue.pop() {
            match task {
                // Handle each task type here
            }
        }
    }

    pub fn spawn_worker(&self) {
        let thread = Thread::new(KERNEL_ASID, Self::do_work);
        let tid = crate::cpu::scheduler::system_scheduler::publish_thread(thread)
            .expect("kernel deferred-worker publication failed");
        SYSTEM_SCHEDULER
            .write()
            .submit_new_thread(tid)
            .expect("Failed to submit DWM worker thread to the system scheduler");
    }
}

impl Default for DeferredWorkManager {
    fn default() -> Self {
        Self::new()
    }
}
