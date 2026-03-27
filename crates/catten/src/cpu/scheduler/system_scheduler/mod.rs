use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;

use spin::Mutex;
use spin::rwlock::RwLock;

use super::lp_schedulers::LpScheduler;
use crate::cpu::isa::constants::interrupt_vectors::LAPIC_TIMER_VECTOR;
use crate::cpu::isa::interface::interrupts::LocalIntCtlrIfce;
use crate::cpu::isa::interrupts::LocalIntCtlr;
use crate::cpu::isa::lp::LpId;
use crate::cpu::isa::lp::ops::get_lp_id;
use crate::cpu::scheduler::threads::ThreadId;
use crate::logln;

pub static SYSTEM_SCHEDULER: RwLock<SystemScheduler> = RwLock::new(SystemScheduler::new());

#[derive(Debug)]
pub enum Error {
    InvalidThread,
}

/// The system-wide thread scheduler
pub struct SystemScheduler {
    lp_schedulers: BTreeMap<LpId, Mutex<Box<dyn LpScheduler>>>,
}

impl SystemScheduler {
    pub const fn new() -> Self {
        Self {
            lp_schedulers: BTreeMap::new(),
        }
    }

    pub unsafe fn set_lp_scheduler(&mut self, lp_sched: Box<dyn LpScheduler>) {
        //! Safety: This function should only be called once per LP at boot during the BSP and AP
        //! init processes and it must be called in the same order that LP IDs were assigned
        //! otherwise the wrong LP will use the wrong local scheduler.
        let ls_sync_ptr = Mutex::new(lp_sched);
        self.lp_schedulers.insert(get_lp_id(), ls_sync_ptr);
    }

    pub fn get_lp_scheduler(&self) -> &Mutex<Box<dyn LpScheduler>> {
        &self.lp_schedulers[&get_lp_id()]
    }

    pub fn submit_ready_thread(&self, tid: ThreadId) -> Result<LpId, Error> {
        logln!("Getting least loaded lp.");
        let least_loaded_lp = self.get_least_loaded_lp();
        logln!("Locking least loaded lp.");
        let was_idle = least_loaded_lp.lock().is_idle();
        logln!("Adding thread to least loaded lp.");
        least_loaded_lp.lock().add_thread(tid).expect("Error adding thread to least loaded LP");
        logln!("Thread added to least loaded lp. Getting LP ID.");
        let lp_id = least_loaded_lp.lock().get_lp_id();
        logln!("LP ID obtained. Returning with ID value.");
        drop(least_loaded_lp.lock());
        if was_idle && lp_id != get_lp_id() {
            logln!("LP {lp_id} was idle, sending wakeup IPI.");
            LocalIntCtlr::send_unicast_ipi(lp_id, LAPIC_TIMER_VECTOR).ok();
        }
        Ok(lp_id)
    }

    // /// Block the specified thread at least until the given event notifies its observers
    // pub fn block_thread(&self, tid: ThreadId, event: &dyn Event) -> Result<(), Error> {
    //     /* Crate a completion object registered with event and push it to the back of the blocker
    //     queue for the specified thread. If the tid doesn't point to any thread structure then
    //     return Error::InvalidThread. If the thread is not already blocked then send a broadcast
    //     over the kernel IPI-RPC protocol with the EvictThread command. */
    //     todo!()
    // }

    // pub fn abort_thread(&self, tid: ThreadId) {
    //     todo!()
    // }

    // pub fn abort_as_threads(&self, asid: AddressSpaceId) {
    //     todo!()
    // }

    fn get_least_loaded_lp(&self) -> &Mutex<Box<dyn LpScheduler>> {
        self.lp_schedulers.iter().min_by_key(|sched| sched.1.lock().thread_count()).unwrap().1
    }
}
