//! System-wide thread scheduler — admission, blocking, abort, and load rebalancing.
//!
//! [`SystemScheduler`] holds one per-LP [`LpScheduler`] and makes global
//! decisions:
//!
//! - **Admission** ([`submit_ready_thread`], [`submit_woken_thread`]): assign a thread to an LP,
//!   preferring its [`affinity_lp`](crate::cpu::scheduler::threads::Thread::affinity_lp) (set at
//!   first admission) over the globally least-loaded LP.
//! - **Blocking** ([`block_thread`], [`block_thread_with_constraint`]): register a waker on an
//!   observable event, transition the thread to `Blocked`, and remove it from the run queue if it
//!   was `Ready`.  Threads that are `Running` (self-block) remain `current_handle` until `next()`
//!   saves their context.
//! - **Abort** ([`abort_thread`]): remove a thread from its LP scheduler and the master table,
//!   stage it for deferred stack reaping.
//! - **Rebalancing** ([`try_rebalance`], [`try_rebalance_sustained`]): periodically migrate
//!   idle-safe threads from overloaded LPs to idle LPs. Only threads explicitly marked
//!   `migration_safe` (no active timers, no pending IPC state) are eligible.  Long-term idle loads
//!   trigger tighter rebalancing through [`try_rebalance_sustained`].

use alloc::{
    boxed::Box,
    collections::btree_map::BTreeMap,
    sync::{
        Arc,
        Weak,
    },
    vec::Vec,
};
use core::sync::atomic::{
    AtomicU64,
    AtomicUsize,
    Ordering,
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
            MigrationConstraint,
            ThreadGeneration,
            ThreadId,
            ThreadState,
            record_exit,
            waker,
        },
    },
    logln,
    memory::AddressSpaceId,
};

const SCHED_TRACE: bool = false;
const REBALANCE_MIN_LOAD_DIFFERENCE: usize = 2;
pub const DEFAULT_REBALANCE_WINDOW_MILLIS: u64 = 100;

macro_rules! sched_trace {
    ($($arg:tt)*) => {
        if SCHED_TRACE {
            logln!($($arg)*);
        }
    };
}

pub static SYSTEM_SCHEDULER: RwLock<SystemScheduler> = RwLock::new(SystemScheduler::new());
pub static REBALANCE_SUCCESSES: AtomicU64 = AtomicU64::new(0);
/// Runtime-adjustable sustained-imbalance window. It is independent of the
/// round-robin quantum; a future policy service may tune it without rebuilding.
pub static REBALANCE_WINDOW_MILLIS: AtomicU64 = AtomicU64::new(DEFAULT_REBALANCE_WINDOW_MILLIS);
pub const MAX_TRACKED_LPS: usize = 256;
pub static LP_LOAD_SUMMARIES: [AtomicUsize; MAX_TRACKED_LPS] =
    [const { AtomicUsize::new(0) }; MAX_TRACKED_LPS];

pub fn set_rebalance_window_millis(window_millis: u64) {
    REBALANCE_WINDOW_MILLIS.store(window_millis.max(1), Ordering::Release);
}

#[derive(Debug)]
pub enum Error {
    InvalidThread,
    AlreadyBlocked,
    ThreadTerminated,
}

/// The system-wide thread scheduler
pub struct SystemScheduler {
    lp_schedulers: BTreeMap<LpId, Mutex<Box<dyn LpScheduler>>>,
    rebalance_pair: AtomicU64,
    rebalance_since_millis: AtomicU64,
}

impl SystemScheduler {
    pub const fn new() -> Self {
        Self {
            lp_schedulers: BTreeMap::new(),
            rebalance_pair: AtomicU64::new(0),
            rebalance_since_millis: AtomicU64::new(0),
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
        {
            let mut table = MASTER_THREAD_TABLE.write();
            let thread = table.get_mut(tid).map_err(|_| Error::InvalidThread)?;
            thread.affinity_lp = Some(target_lp);
            thread.pinned_lp = Some(target_lp);
            thread.migration_safe = false;
        }
        sched_guard.set_ctx_switch_pending();
        drop(sched_guard);
        if target_lp != get_lp_id() {
            LocalIntCtlr::send_unicast_ipi(target_lp, SCHEDULER_IPI_VECTOR)
                .expect("failed to send scheduler wake IPI");
        }
        Ok(())
    }

    /// Give certified migratable work an initial soft placement. Unlike
    /// `submit_to_lp`, this does not pin the thread; it exists for deliberate
    /// initial placement and for the boot rebalancing regression workload.
    pub fn submit_migratable_to_lp(&self, tid: ThreadId, target_lp: LpId) -> Result<(), Error> {
        let sched = self.lp_schedulers.get(&target_lp).ok_or(Error::InvalidThread)?;
        let mut sched_guard = sched.lock();
        sched_guard.add_thread(tid, None).map_err(|_| Error::InvalidThread)?;
        {
            let mut table = MASTER_THREAD_TABLE.write();
            let thread = table.get_mut(tid).map_err(|_| Error::InvalidThread)?;
            if !thread.migration_safe || thread.pinned_lp.is_some() {
                return Err(Error::InvalidThread);
            }
            thread.affinity_lp = Some(target_lp);
        }
        sched_guard.set_ctx_switch_pending();
        drop(sched_guard);
        if target_lp != get_lp_id() {
            LocalIntCtlr::send_unicast_ipi(target_lp, SCHEDULER_IPI_VECTOR)
                .expect("failed to send scheduler wake IPI");
        }
        Ok(())
    }

    /// Move at most one explicitly migratable Ready thread from the busiest LP
    /// to the least-loaded LP. Running and Blocked threads never migrate: a
    /// Blocked thread may still own an event in its affinity LP's timer queue,
    /// while a Running thread's context has not completed the `on_cpu`
    /// hand-off. Both LP queues are locked in numeric order before the thread
    /// table, making the queue move, state transition, and affinity update one
    /// transaction under the scheduler's canonical lock order.
    pub fn try_rebalance(&self) -> bool {
        if self.lp_schedulers.len() < 2 {
            return false;
        }

        let mut loads: Vec<(LpId, usize)> = self
            .lp_schedulers
            .keys()
            .map(|&lp| {
                assert!((lp as usize) < MAX_TRACKED_LPS);
                (lp, LP_LOAD_SUMMARIES[lp as usize].load(Ordering::Acquire))
            })
            .collect();
        loads.sort_unstable_by_key(|&(lp, load)| (load, lp));
        let (destination_lp, destination_load) = loads[0];
        let (source_lp, source_load) = loads[loads.len() - 1];
        if source_lp == destination_lp
            || source_load < destination_load + REBALANCE_MIN_LOAD_DIFFERENCE
        {
            return false;
        }

        let first_lp = source_lp.min(destination_lp);
        let second_lp = source_lp.max(destination_lp);
        let mut first = self.lp_schedulers[&first_lp].lock();
        let mut second = self.lp_schedulers[&second_lp].lock();
        let (source, destination): (&mut dyn LpScheduler, &mut dyn LpScheduler) =
            if source_lp == first_lp {
                (&mut **first, &mut **second)
            } else {
                (&mut **second, &mut **first)
            };

        // Loads may have changed while the two locks were acquired.
        if source.thread_count() < destination.thread_count() + REBALANCE_MIN_LOAD_DIFFERENCE {
            return false;
        }

        let candidates = source.ready_migration_candidates();
        let mut table = MASTER_THREAD_TABLE.write();
        let candidate = candidates.into_iter().find(|&(tid, generation)| {
            table.get(tid).is_ok_and(|thread| {
                thread.generation == generation
                    && thread.is_fully_migratable()
                    && matches!(thread.state, ThreadState::Ready(lp) if lp == source_lp)
            })
        });
        let Some((tid, generation)) = candidate else {
            return false;
        };

        source
            .remove_ready_for_migration(tid, generation)
            .expect("validated migration candidate vanished from source queue");
        destination
            .add_ready_from_migration(tid, generation)
            .expect("migration duplicated a destination queue entry");
        let thread = table.get_mut(tid).expect("validated migration candidate vanished");
        thread.state = ThreadState::Ready(destination_lp);
        thread.affinity_lp = Some(destination_lp);
        destination.set_ctx_switch_pending();
        drop(table);
        drop(second);
        drop(first);

        if destination_lp != get_lp_id() {
            LocalIntCtlr::send_unicast_ipi(destination_lp, SCHEDULER_IPI_VECTOR)
                .expect("failed to send scheduler rebalance IPI");
        }
        crate::debug_trace::trace(
            crate::debug_trace::TAG_SCHED_ADMIT,
            tid as u64,
            generation,
            (1u64 << 63) | destination_lp as u64,
        );
        REBALANCE_SUCCESSES.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Low-pass-filtered runtime rebalance entry point. A transient load spike
    /// merely starts/replaces the observation window. Only the same busiest
    /// and least-loaded LP pair remaining imbalanced for the full configured
    /// window reaches the transactional migration path.
    ///
    /// This is intentionally not called from wake admission. A future runtime
    /// sampler should invoke it from a non-interrupt scheduler maintenance
    /// point with a monotonic millisecond timestamp.
    pub fn try_rebalance_sustained(&self, now_millis: u64) -> bool {
        if self.lp_schedulers.len() < 2 {
            return false;
        }
        let mut loads: Vec<(LpId, usize)> = self
            .lp_schedulers
            .keys()
            .map(|&lp| {
                assert!((lp as usize) < MAX_TRACKED_LPS);
                (lp, LP_LOAD_SUMMARIES[lp as usize].load(Ordering::Acquire))
            })
            .collect();
        loads.sort_unstable_by_key(|&(lp, load)| (load, lp));
        let (destination_lp, destination_load) = loads[0];
        let (source_lp, source_load) = loads[loads.len() - 1];
        if source_lp == destination_lp
            || source_load < destination_load + REBALANCE_MIN_LOAD_DIFFERENCE
        {
            self.rebalance_pair.store(0, Ordering::Release);
            return false;
        }

        let pair = 1 + ((source_lp as u64) << 32) + destination_lp as u64;
        if self.rebalance_pair.load(Ordering::Acquire) != pair {
            self.rebalance_since_millis.store(now_millis, Ordering::Relaxed);
            self.rebalance_pair.store(pair, Ordering::Release);
            return false;
        }
        let since = self.rebalance_since_millis.load(Ordering::Relaxed);
        if now_millis.saturating_sub(since) < REBALANCE_WINDOW_MILLIS.load(Ordering::Acquire) {
            return false;
        }

        self.rebalance_pair.store(0, Ordering::Release);
        self.try_rebalance()
    }

    /// Block the specified thread at least until the given event notifies its observers
    pub fn block_thread<'a>(
        &self,
        tid: ThreadId,
        event: &'a dyn crate::klib::observer::Observable,
    ) -> Result<(), Error> {
        self.block_thread_with_constraint(tid, event, MigrationConstraint::GeneralWait)
    }

    pub fn block_thread_with_constraint<'a>(
        &self,
        tid: ThreadId,
        event: &'a dyn crate::klib::observer::Observable,
        constraint: MigrationConstraint,
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
        thread.add_migration_constraint(constraint);
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
        record_exit(stage_lp, tid, thread.generation);
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
            match self.abort_thread(tid) {
                Ok(_) | Err(Error::InvalidThread) => {}
                Err(error) => panic!("Error aborting thread by ASID: {:?}", error),
            }
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
