//! # Deferred Work Manager
//!
//! Interrupt service routines do not ever block or spin as both are deadlock hazards. Instead they
//! submit work to the Deferred Work Manager, which is a lock-free queue. The Deferred Work Manager
//! is then responsible for ensuring the work is completed. Since most if not all work submitted
//! will come from interrupt context, the DWM does not ever attempt to return a status or any data
//! back to the calling context which is assumed to no longer exist.
//!
//! The DWM is a threadpool with a lockless MPMC queue which accepts predefined tasks which include
//! their arguments. The DWM executes the tasks without any ordering guarantees however it does
//! guarantee that a task will be executed at some point in the future. It is optimized to maximize
//! throughput and makes no latency guarantees. For latency sensitive tasks a dedicated kernel
//! thread should be used instead.

use concurrent_queue::ConcurrentQueue;
use spin::LazyLock;

use crate::{
    cpu::scheduler::{
        system_scheduler::SYSTEM_SCHEDULER,
        threads::{
            MASTER_THREAD_TABLE,
            Thread,
        },
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
        while let Ok(task) = DWM.queue.pop() {
            match task {
                // Handle each task type here
            }
        }
    }

    pub fn spawn_worker(&self) {
        let thread = Thread::new(KERNEL_ASID, Self::do_work);
        let tid = MASTER_THREAD_TABLE.write().add_element(thread);
        SYSTEM_SCHEDULER
            .write()
            .submit_ready_thread(tid)
            .expect("Failed to submit DWM worker thread to the system scheduler");
    }
}
