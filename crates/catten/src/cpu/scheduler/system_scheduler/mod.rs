use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::format;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use crate::cpu::multiprocessor::spin::mutex::Mutex;
use spin::rwlock::RwLock;

use super::lp_schedulers::LpScheduler;
use crate::cpu::isa::constants::interrupt_vectors::LAPIC_TIMER_VECTOR;
use crate::cpu::isa::interface::interrupts::LocalIntCtlrIfce;
use crate::cpu::isa::interrupts::LocalIntCtlr;
use crate::cpu::isa::lp::LpId;
use crate::cpu::isa::lp::ops::get_lp_id;
use crate::cpu::scheduler::threads::{MASTER_THREAD_TABLE, ThreadId, ThreadState, waker};
use crate::logln;
use crate::memory::AddressSpaceId;

pub static SYSTEM_SCHEDULER: RwLock<SystemScheduler> = RwLock::new(SystemScheduler::new());

#[derive(Debug)]
pub enum Error {
    InvalidThread,
    AlreadyBlocked,
    ThreadTerminated,
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
        if was_idle && lp_id != get_lp_id() {
            logln!("LP {lp_id} was idle, sending wakeup IPI.");
            LocalIntCtlr::send_unicast_ipi(lp_id, LAPIC_TIMER_VECTOR).ok();
        }
        Ok(lp_id)
    }

    /// Block the specified thread at least until the given event notifies its observers
    pub fn block_thread<'a>(
        &mut self,
        tid: ThreadId,
        event: &'a mut dyn crate::klib::observer::Observable,
    ) -> Result<(), Error> {
        if let Ok(thread) = MASTER_THREAD_TABLE.write().get_mut(tid) {
            match thread.state {
                ThreadState::Running(lp_id) | ThreadState::Ready(lp_id) => {
                    self.lp_schedulers[&lp_id]
                        .lock()
                        .remove_thread(tid)
                        .expect("Error removing thread from LP scheduler while blocking");
                }
                ThreadState::NeedsLpAssignment => {}
                ThreadState::Blocked(_) => {
                    return Err(Error::AlreadyBlocked);
                }
            }
            let waker = Arc::new(waker::Waker::new(tid));
            event.register_observer(
                Arc::downgrade(&waker) as Weak<dyn crate::klib::observer::Observer>
            );
            thread.state = ThreadState::Blocked(waker);
            Ok(())
        } else {
            Err(Error::InvalidThread)
        }
    }

    pub fn abort_thread(&self, tid: ThreadId) -> Result<ThreadId, Error> {
        if let Ok(thread) = MASTER_THREAD_TABLE.write().get_mut(tid) {
            match thread.state {
                ThreadState::Running(lp_id) | ThreadState::Ready(lp_id) => {
                    self.lp_schedulers[&lp_id]
                        .lock()
                        .remove_thread(tid)
                        .expect("Error removing thread from LP scheduler while aborting");
                }
                _ => {}
            }
            MASTER_THREAD_TABLE
                .write()
                .remove_element(tid)
                .expect(&format!("Failed to delete thread {tid}"));
            Ok(tid)
        } else {
            Err(Error::InvalidThread)
        }
    }

    pub fn abort_as_threads(&self, asid: AddressSpaceId) {
        let mut threads_to_abort = Vec::new();
        for (id, thread) in MASTER_THREAD_TABLE.read().iter().enumerate() {
            if let Some(thread) = thread {
                if thread.asid == asid {
                    threads_to_abort.push(id);
                }
            }
        }
        for tid in threads_to_abort {
            self.abort_thread(tid).expect("Error aborting thread by ASID");
        }
    }

    fn get_least_loaded_lp(&self) -> &Mutex<Box<dyn LpScheduler>> {
        self.lp_schedulers.iter().min_by_key(|sched| sched.1.lock().thread_count()).unwrap().1
    }
}
