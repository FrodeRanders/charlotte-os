use alloc::sync::Arc;
use alloc::vec::Vec;

use spin::Mutex;
use spin::rwlock::RwLock;

use super::lp_schedulers::LpScheduler;
use crate::cpu::isa::lp::LpId;
use crate::cpu::isa::lp::ops::{get_lp_id, yield_lp};
use crate::cpu::scheduler::threads::ThreadId;
use crate::event::Event;
use crate::memory::AddressSpaceId;

pub static SYSTEM_SCHEDULER: RwLock<SystemScheduler> = RwLock::new(SystemScheduler::new());

pub enum Error {
    InvalidThread,
}

/// The system-wide thread scheduler
pub struct SystemScheduler {
    lp_schedulers: Vec<Arc<Mutex<dyn LpScheduler>>>,
}

impl SystemScheduler {
    pub const fn new() -> Self {
        Self {
            lp_schedulers: Vec::new(),
        }
    }

    pub unsafe fn set_lp_scheduler(&mut self, lp_sched: &dyn LpScheduler) {
        //! Safety: This function should only be called once per LP at boot during the BSP and AP
        //! init processes and it must be called in the same order that LP IDs were assigned
        //! otherwise the wrong LP will use the wrong local scheduler.
        let ls_sync_ptr = Arc::new(Mutex::new(*lp_sched));
        self.lp_schedulers.push(ls_sync_ptr);
    }

    pub fn get_lp_scheduler(&self) -> Arc<Mutex<LpScheduler>> {
        self.lp_schedulers[get_lp_id() as usize].clone()
    }

    pub fn submit_ready_thread(&self, tid: ThreadId) -> Result<LpId, Error> {
        let least_loaded_lp = self.get_least_loaded_lp();
        least_loaded_lp.lock().add_thread(tid);
        Ok(least_loaded_lp.lock().lp_id)
    }

    pub unsafe fn yield_lp(&self) -> ! {
        //! Yield the current LP's execution to the scheduler
        //! This differs from blocking in that the processor state on entry is discarded
        yield_lp!()
    }

    /// Block the specified thread at least until the given event notifies its observers
    pub fn block_thread(&self, tid: ThreadId, event: &dyn Event) -> Result<(), Error> {
        /* Crate a completion object registered with event and push it to the back of the blocker
        queue for the specified thread. If the tid doesn't point to any thread structure then
        return Error::InvalidThread. If the thread is not already blocked then send a broadcast
        over the kernel IPI-RPC protocol with the EvictThread command. */
        todo!()
    }

    pub fn abort_thread(&self, tid: ThreadId) {
        todo!()
    }

    pub fn abort_as_threads(&self, asid: AddressSpaceId) {
        todo!()
    }

    fn get_least_loaded_lp(&self) -> Arc<Mutex<LpScheduler>> {
        self.lp_schedulers.iter().min_by_key(|sched| sched.lock().thread_count()).unwrap().clone()
    }
}
