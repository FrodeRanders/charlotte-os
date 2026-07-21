use alloc::{
    boxed::Box,
    collections::btree_map::BTreeMap,
    sync::{
        Arc,
        Weak,
    },
    vec::Vec,
};

use super::lp_schedulers::LpScheduler;
use crate::{
    cpu::{
        isa::{
            constants::interrupt_vectors::SCHEDULER_IPI_VECTOR,
            interface::interrupts::LocalIntCtlrIfce,
            interrupts::LocalIntCtlr,
            lp::{
                LpId,
                ops::get_lp_id,
            },
        },
        multiprocessor::spin::{
            mutex::Mutex,
            rwlock::RwLock,
        },
        scheduler::threads::{
            MASTER_THREAD_TABLE,
            ThreadGeneration,
            ThreadId,
            ThreadState,
            waker,
        },
    },
    logln,
    memory::AddressSpaceId,
};

const SCHED_TRACE: bool = false;

macro_rules! sched_trace {
    ($($arg:tt)*) => {
        if SCHED_TRACE {
            logln!($($arg)*);
        }
    };
}

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

    /// Pick the LP to admit a thread to: if the thread already has an
    /// affinity, prefer it; otherwise pick the least-loaded LP.
    fn pick_lp_for(&self, tid: ThreadId) -> &Mutex<Box<dyn LpScheduler>> {
        // Check if the thread already has an affinity LP (read-only, no lock).
        let existing = {
            let table = MASTER_THREAD_TABLE.read();
            table.get(tid).ok().and_then(|t| t.affinity_lp)
        };
        if let Some(lp) = existing {
            if let Some(sched) = self.lp_schedulers.get(&lp) {
                return sched;
            }
        }
        self.get_least_loaded_lp()
    }

    pub fn submit_ready_thread(&self, tid: ThreadId) -> Result<LpId, Error> {
        let target = self.pick_lp_for(tid);
        let mut lp_guard = target.lock();
        let load_before = lp_guard.thread_count();
        match lp_guard.add_thread(tid, None) {
            Ok(()) => {}
            Err(_) => return Err(Error::InvalidThread),
        }
        let lp_id = lp_guard.get_lp_id();
        let load_after = lp_guard.thread_count();
        // Admission is itself a scheduling event. This is required for
        // same-LP admission (which sends no IPI), and harmlessly coalesces for
        // duplicate wakes or a remote admission whose IPI also sets pending.
        lp_guard.set_ctx_switch_pending();
        // Set affinity on first assignment — do this under the LP scheduler
        // lock to honour the lp_scheduler → MASTER_THREAD_TABLE lock order.
        {
            let mut table = MASTER_THREAD_TABLE.write();
            if let Ok(thread) = table.get_mut(tid) {
                if thread.affinity_lp.is_none() {
                    thread.affinity_lp = Some(lp_id);
                }
            }
        }
        drop(lp_guard);
        sched_trace!(
            "[sched] submit_ready TID={} -> LP{} load={}->{}",
            tid,
            lp_id,
            load_before,
            load_after
        );
        if lp_id != get_lp_id() {
            sched_trace!("[sched]   IPI -> LP{} for TID={}", lp_id, tid);
            LocalIntCtlr::send_unicast_ipi(lp_id, SCHEDULER_IPI_VECTOR)
                .expect("failed to send scheduler wake IPI");
        }
        Ok(lp_id)
    }

    pub fn submit_woken_thread(
        &self,
        tid: ThreadId,
        generation: ThreadGeneration,
    ) -> Result<LpId, Error> {
        let target = self.pick_lp_for(tid);
        let mut lp_guard = target.lock();
        let load_before = lp_guard.thread_count();
        lp_guard.add_thread(tid, Some(generation)).map_err(|_| Error::InvalidThread)?;
        let lp_id = lp_guard.get_lp_id();
        let load_after = lp_guard.thread_count();
        lp_guard.set_ctx_switch_pending();
        drop(lp_guard);
        sched_trace!(
            "[sched] submit_woken TID={} gen={} -> LP{} load={}->{}",
            tid,
            generation,
            lp_id,
            load_before,
            load_after
        );
        if lp_id != get_lp_id() {
            sched_trace!("[sched]   IPI -> LP{} for TID={}", lp_id, tid);
            LocalIntCtlr::send_unicast_ipi(lp_id, SCHEDULER_IPI_VECTOR)
                .expect("failed to send scheduler wake IPI");
        }
        Ok(lp_id)
    }

    /// Submit a thread to a specific LP, pinning it there. Used by
    /// `ShardRuntime::spawn_shard` to bind a sitas shard to a core.
    pub fn submit_to_lp(&self, tid: ThreadId, target_lp: LpId) -> Result<(), Error> {
        let sched = match self.lp_schedulers.get(&target_lp) {
            Some(s) => s,
            None => {
                let n = self.lp_schedulers.len();
                logln!("submit_to_lp: LP {target_lp} not found (lp_schedulers has {n} entries)");
                return Err(Error::InvalidThread);
            }
        };
        let mut sched_guard = sched.lock();
        sched_guard.add_thread(tid, None).expect("Error adding thread to target LP");
        sched_guard.set_ctx_switch_pending();
        drop(sched_guard);
        if target_lp != get_lp_id() {
            LocalIntCtlr::send_unicast_ipi(target_lp, SCHEDULER_IPI_VECTOR)
                .expect("failed to send scheduler wake IPI");
        }
        Ok(())
    }

    /// Block the specified thread at least until the given event notifies its observers
    pub fn block_thread<'a>(
        &self,
        tid: ThreadId,
        event: &'a dyn crate::klib::observer::Observable,
    ) -> Result<(), Error> {
        let state = {
            let table = MASTER_THREAD_TABLE.read();
            table
                .get(tid)
                .map(|thread| match thread.state {
                    ThreadState::Running(_) => ThreadStateSnapshot::Running,
                    ThreadState::Ready(lp) => ThreadStateSnapshot::Ready(lp),
                    ThreadState::NeedsLpAssignment => ThreadStateSnapshot::NeedsLpAssignment,
                    ThreadState::Blocked(_) => ThreadStateSnapshot::Blocked,
                })
                .map_err(|_| Error::InvalidThread)?
        };

        // For a queued thread, honor the global LP-scheduler -> thread-table
        // lock order used by dispatch, wake, and abort. Holding these in the
        // reverse order can deadlock an LP in `RoundRobin::next`.
        let mut lp_guard = match state {
            ThreadStateSnapshot::Ready(lp_id) => Some(self.lp_schedulers[&lp_id].lock()),
            _ => None,
        };
        let mut table = MASTER_THREAD_TABLE.write();
        let thread = table.get_mut(tid).map_err(|_| Error::InvalidThread)?;
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
                let guard = lp_guard.as_mut().ok_or(Error::InvalidThread)?;
                if guard.get_lp_id() != lp_id {
                    return Err(Error::InvalidThread);
                }
                guard
                    .remove_thread(tid)
                    .expect("Error removing thread from LP scheduler while blocking");
            }
            ThreadState::NeedsLpAssignment => {}
            ThreadState::Blocked(_) => {
                return Err(Error::AlreadyBlocked);
            }
        }
        let generation = thread.generation;
        let waker = Arc::new(waker::Waker::new(tid, generation));
        event
            .register_observer(Arc::downgrade(&waker) as Weak<dyn crate::klib::observer::Observer>);
        thread.state = ThreadState::Blocked(waker);
        Ok(())
    }

    pub fn abort_thread(&self, tid: ThreadId) -> Result<ThreadId, Error> {
        // Determine where the thread is known to be under a short-lived read
        // lock, so we do NOT hold a MASTER_THREAD_TABLE guard across the later
        // write lock (doing so would self-deadlock the non-reentrant RwLock).
        let state_lp = {
            let table = MASTER_THREAD_TABLE.read();
            match table.get(tid) {
                Ok(thread) => match thread.state {
                    ThreadState::Running(lp_id) | ThreadState::Ready(lp_id) => Some(lp_id),
                    _ => None,
                },
                Err(_) => return Err(Error::InvalidThread),
            }
        };
        let current_lp = state_lp.or_else(|| self.current_lp_for_thread(tid));
        let remove_lp = state_lp.or(current_lp);
        if let Some(lp_id) = remove_lp {
            self.lp_schedulers[&lp_id]
                .lock()
                .remove_thread(tid)
                .expect("Error removing thread from LP scheduler while aborting");
        }
        // Move the thread out of the table WITHOUT dropping it: a thread cannot
        // free its own kernel stack while still executing on it. Stage it for
        // reaping on the LP it last ran on (its own LP for a self-abort). That
        // LP's `reap_dead_threads` (called from `cond_yield_lp` after switching
        // away) drops it later, once it is guaranteed off its stack. Staging it
        // on any other LP would risk freeing a stack still in use.
        let stage_lp = current_lp.unwrap_or_else(get_lp_id);
        let thread =
            MASTER_THREAD_TABLE.write().take_element(tid).map_err(|_| Error::InvalidThread)?;
        crate::cpu::scheduler::threads::stage_dead_thread(stage_lp, thread);
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

    fn current_lp_for_thread(&self, tid: ThreadId) -> Option<LpId> {
        self.lp_schedulers.iter().find_map(|(&lp_id, sched)| {
            if sched.lock().get_tid() == Some(tid) {
                Some(lp_id)
            } else {
                None
            }
        })
    }
}

#[derive(Clone, Copy)]
enum ThreadStateSnapshot {
    Running,
    Ready(LpId),
    NeedsLpAssignment,
    Blocked,
}

pub fn get_thread_id() -> Option<ThreadId> {
    SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid()
}
