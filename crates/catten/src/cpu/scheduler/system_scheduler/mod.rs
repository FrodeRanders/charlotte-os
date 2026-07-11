use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use super::lp_schedulers::LpScheduler;
use crate::cpu::isa::constants::interrupt_vectors::LAPIC_TIMER_VECTOR;
use crate::cpu::isa::interface::interrupts::LocalIntCtlrIfce;
use crate::cpu::isa::interrupts::LocalIntCtlr;
use crate::cpu::isa::lp::LpId;
use crate::cpu::isa::lp::ops::get_lp_id;
use crate::cpu::multiprocessor::spin::mutex::Mutex;
use crate::cpu::multiprocessor::spin::rwlock::RwLock;
use crate::cpu::scheduler::threads::{DEAD_THREADS, MASTER_THREAD_TABLE, ThreadId, ThreadState, waker};
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
        event: &'a dyn crate::klib::observer::Observable,
    ) -> Result<(), Error> {
        if let Ok(thread) = MASTER_THREAD_TABLE.write().get_mut(tid) {
            match thread.state {
                ThreadState::Running(_) => {
                    // The thread is currently executing on its LP and is
                    // blocking itself. Do NOT remove it from the LP scheduler
                    // yet: it must remain the LP's `current_handle` so that the
                    // following `cond_yield_lp` saves its execution context to
                    // its own `saved_sp`. `RoundRobin::next` declines to
                    // re-queue a Blocked thread, so it will not be rescheduled
                    // until its waker fires and re-admits it.
                }
                ThreadState::Ready(lp_id) => {
                    // Queued but not running: pull it out of the run queue.
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
        // Determine which LP the thread is assigned to under a short-lived read
        // lock, so we do NOT hold a MASTER_THREAD_TABLE guard across the later
        // write lock (doing so would self-deadlock the non-reentrant RwLock).
        let lp = {
            let table = MASTER_THREAD_TABLE.read();
            match table.get(tid) {
                Ok(thread) => match thread.state {
                    ThreadState::Running(lp_id) | ThreadState::Ready(lp_id) => Some(lp_id),
                    _ => None,
                },
                Err(_) => return Err(Error::InvalidThread),
            }
        };
        if let Some(lp_id) = lp {
            self.lp_schedulers[&lp_id]
                .lock()
                .remove_thread(tid)
                .expect("Error removing thread from LP scheduler while aborting");
        }
        // Move the thread out of the table WITHOUT dropping it: a thread cannot
        // free its own kernel stack while still executing on it. The reaper
        // (`reap_dead_threads`, called from `cond_yield_lp` after switching away)
        // drops it later from another thread's context.
        let thread = MASTER_THREAD_TABLE
            .write()
            .take_element(tid)
            .map_err(|_| Error::InvalidThread)?;
        DEAD_THREADS.write().push(thread);
        Ok(tid)
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

pub fn get_thread_id() -> Option<ThreadId> {
    SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid()
}
