//! System-wide thread scheduler — admission, blocking, abort, and load rebalancing.
//!
//! [`SystemScheduler`] holds one per-LP [`LpScheduler`] and makes global
//! decisions:
//!
//! - **Admission** ([`submit_new_thread`], [`submit_woken_thread`]): assign a thread to an LP,
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
    sync::Arc,
};
use core::sync::atomic::{
    AtomicU64,
    AtomicUsize,
    Ordering,
};

mod domain_abort;
mod selection_tests;
mod wait;
use wait::with_detached_wait;
pub(crate) fn test_rebalance_selection() {
    selection_tests::run();
    super::lp_schedulers::round_robin::test_migration_selection();
    wait::test_publication();
}
pub(crate) use domain_abort::{
    abort_domain_threads,
    abort_domain_threads_with_request,
};
pub(crate) fn test_domain_abort() {
    domain_abort::tests::run();
}
pub(crate) use domain_abort::tests::{
    arm_concurrent_handoff,
    arm_self_handoff,
    finish_concurrent_handoff,
    finish_self_handoff,
    verify_executor_rejection,
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
            Thread,
            ThreadGeneration,
            ThreadId,
            ThreadState,
            begin_retirement,
            record_exit,
            waker,
        },
    },
    logln,
};

const SCHED_TRACE: bool = false;
const REBALANCE_MIN_LOAD_DIFFERENCE: usize = 2;
pub const DEFAULT_REBALANCE_WINDOW_MILLIS: u64 = 100;

/// Preserve the former `(load, LP)` sorting order without an allocated
/// snapshot. Each summary is sampled once; actual queues are rechecked under
/// both LP locks before a move, exactly as for the former snapshot.
#[derive(Debug, PartialEq, Eq)]
struct LoadExtremes {
    least: (LpId, usize),
    most: (LpId, usize),
}

impl LoadExtremes {
    fn sample(loads: impl IntoIterator<Item = (LpId, usize)>) -> Option<Self> {
        let mut loads = loads.into_iter();
        let first = loads.next()?;
        let mut result = Self {
            least: first,
            most: first,
        };
        for (lp, load) in loads {
            if (load, lp) < (result.least.1, result.least.0) {
                result.least = (lp, load);
            }
            if (load, lp) > (result.most.1, result.most.0) {
                result.most = (lp, load);
            }
        }
        Some(result)
    }
}

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

/// Serializes thread publication with the exact root's inline abort fence.
/// No map or per-abort storage is created while teardown is in progress.
static THREAD_PUBLICATION_GATE: Mutex<()> = Mutex::new(());

pub(crate) fn thread_admission_open(handle: crate::memory::AddressSpaceHandle) -> bool {
    let table = crate::memory::ADDRESS_SPACE_TABLE.lock();
    table.generation(handle.id()).ok() == Some(handle.generation())
        && table.is_closing(handle.id()) == Ok(false)
        && table.get(handle.id()).is_ok_and(|space| !space.thread_admission_closed)
}

/// Publish a newly constructed thread unless its address-space lifetime is
/// already aborting.
///
/// Publication and installing the root's abort fence use the same gate. A
/// thread is either published before the fence or rejected afterward.
#[allow(clippy::result_large_err)] // Carry the complete rejected owner out without allocation.
pub fn publish_thread(thread: Thread) -> Result<ThreadId, Error> {
    // Carry rejected payload ownership out of every serialization scope.
    let publication = (|| {
        let asid = thread.asid;
        let _lifecycle = (asid != crate::memory::KERNEL_ASID)
            .then(|| crate::memory::ADDRESS_SPACE_LIFECYCLE.lock());
        let _publication_gate = THREAD_PUBLICATION_GATE.lock();
        let maximum = if asid != crate::memory::KERNEL_ASID {
            let Some(handle) = thread.address_space else {
                return Err((thread, Error::ThreadTerminated));
            };
            if handle.id() != asid || !thread_admission_open(handle) {
                return Err((thread, Error::ThreadTerminated));
            }
            Some(crate::memory::domain_limits(asid).max_threads)
        } else {
            None
        };
        let mut table = MASTER_THREAD_TABLE.write();
        if maximum.is_some_and(|maximum| {
            table
                .iter()
                .filter(|entry| entry.as_ref().is_some_and(|thread| thread.asid == asid))
                .count()
                >= maximum
        }) {
            return Err((thread, Error::DomainThreadLimitExceeded));
        }
        table
            .try_add_element(thread)
            .map_err(|(thread, _)| (thread, Error::ThreadPreparationFailed))
    })();
    publication.map_err(|(thread, error)| {
        // Physical rejection retains backing/admission; never invoke it again
        // through Drop. Outer callers' IRQ state is preserved by the adapters.
        let _ = thread.release_unstarted();
        error
    })
}

/// Exact-generation process liveness for the trusted remote-resource adapter.
/// Abort fencing and publication use this same gate before the thread table.
pub(crate) fn domain_has_live_threads(handle: crate::memory::AddressSpaceHandle) -> bool {
    let _publication_gate = THREAD_PUBLICATION_GATE.lock();
    if !thread_admission_open(handle) {
        return false;
    }
    MASTER_THREAD_TABLE
        .read()
        .iter()
        .any(|entry| entry.as_ref().is_some_and(|thread| thread.address_space == Some(handle)))
}

pub fn set_rebalance_window_millis(window_millis: u64) {
    REBALANCE_WINDOW_MILLIS.store(window_millis.max(1), Ordering::Release);
}

#[derive(Debug)]
pub enum Error {
    InvalidThread,
    AlreadyBlocked,
    DomainThreadLimitExceeded,
    ThreadTerminated,
    WaitRegistrationFailed,
    PermissionDenied,
    ThreadPreparationFailed,
    ThreadRetirementFailed,
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

    /// # Safety
    ///
    /// Call exactly once per LP during BSP/AP initialization, after that LP's
    /// logical ID has been assigned.
    pub unsafe fn set_lp_scheduler(&mut self, lp_sched: Box<dyn LpScheduler>) {
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
            table.get(tid).ok().and_then(|t| {
                let executor = t.operation_executor_lp.load(Ordering::Acquire);
                if executor != usize::MAX {
                    Some(executor as LpId)
                } else {
                    t.affinity_lp
                }
            })
        };
        if let Some(lp) = existing
            && let Some(sched) = self.lp_schedulers.get(&lp)
        {
            return sched;
        }
        self.get_least_loaded_lp()
    }

    /// Admit a thread immediately after inserting it in the master table.
    ///
    /// This generation-free operation is intentionally restricted to initial
    /// admission. Any reference retained across blocking, notification, or
    /// asynchronous work must use [`Self::submit_woken_thread`] instead.
    pub fn submit_new_thread(&self, tid: ThreadId) -> Result<LpId, Error> {
        if !MASTER_THREAD_TABLE
            .read()
            .get(tid)
            .is_ok_and(|thread| matches!(thread.state, ThreadState::NeedsLpAssignment))
        {
            return Err(Error::InvalidThread);
        }
        let (lp_id, load_before, load_after) = with_detached_wait(|detached| {
            let target = self.pick_lp_for(tid);
            let mut lp_guard = target.lock();
            let load_before = lp_guard.thread_count();
            lp_guard.add_thread(tid, None, None, detached).map_err(|_| Error::InvalidThread)?;
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
                if let Ok(thread) = table.get_mut(tid)
                    && thread.affinity_lp.is_none()
                {
                    thread.affinity_lp = Some(lp_id);
                }
            }
            Ok((lp_id, load_before, load_after))
        })?;
        sched_trace!(
            "[sched] submit_new TID={} -> LP{} load={}->{}",
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
        self.submit_woken_wait_matching(tid, generation, None)
    }

    /// Source callbacks must match the installed wait as well as the thread
    /// lifetime. A delayed hint from an earlier park cannot wake a later one.
    pub(crate) fn submit_woken_wait(
        &self,
        tid: ThreadId,
        generation: ThreadGeneration,
        waker: &waker::Waker,
    ) -> Result<LpId, Error> {
        self.submit_woken_wait_matching(tid, generation, Some(waker))
    }

    fn submit_woken_wait_matching(
        &self,
        tid: ThreadId,
        generation: ThreadGeneration,
        expected_waker: Option<&waker::Waker>,
    ) -> Result<LpId, Error> {
        let (lp_id, load_before, load_after) = with_detached_wait(|detached| {
            let target = self.pick_lp_for(tid);
            let mut lp_guard = target.lock();
            let load_before = lp_guard.thread_count();
            lp_guard
                .add_thread(tid, Some(generation), expected_waker, detached)
                .map_err(|_| Error::InvalidThread)?;
            let lp_id = lp_guard.get_lp_id();
            let load_after = lp_guard.thread_count();
            lp_guard.set_ctx_switch_pending();
            Ok((lp_id, load_before, load_after))
        })?;
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
        with_detached_wait(|detached| {
            let mut sched_guard = sched.lock();
            sched_guard.add_thread(tid, None, None, detached).map_err(|_| Error::InvalidThread)?;
            {
                let mut table = MASTER_THREAD_TABLE.write();
                let thread = table.get_mut(tid).map_err(|_| Error::InvalidThread)?;
                thread.affinity_lp = Some(target_lp);
                thread.pinned_lp = Some(target_lp);
                thread.migration_safe = false;
            }
            sched_guard.set_ctx_switch_pending();
            Ok(())
        })?;
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
        with_detached_wait(|detached| {
            let mut sched_guard = sched.lock();
            sched_guard.add_thread(tid, None, None, detached).map_err(|_| Error::InvalidThread)?;
            {
                let mut table = MASTER_THREAD_TABLE.write();
                let thread = table.get_mut(tid).map_err(|_| Error::InvalidThread)?;
                if !thread.migration_safe || thread.pinned_lp.is_some() {
                    return Err(Error::InvalidThread);
                }
                thread.affinity_lp = Some(target_lp);
            }
            sched_guard.set_ctx_switch_pending();
            Ok(())
        })?;
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

        let loads = self.load_extremes().expect("multiple LP schedulers have no load summaries");
        let (destination_lp, destination_load) = loads.least;
        let (source_lp, source_load) = loads.most;
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

        let mut table = MASTER_THREAD_TABLE.write();
        let candidate = source.ready_migration_candidate(&table);
        let Ok(Some((tid, generation))) = candidate else {
            return false;
        };

        if source.migrate_ready_to(destination, &mut table, tid, generation).is_err() {
            return false;
        }
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
    /// Current callers are the architecture yield tails, after switch guards
    /// leave but before restoring the caller's IRQ state. Selection therefore
    /// cannot allocate. Queue storage is inline in already admitted threads;
    /// broader waiter, quantum-timer and caller contexts remain separate.
    pub fn try_rebalance_sustained(&self, now_millis: u64) -> bool {
        if self.lp_schedulers.len() < 2 {
            return false;
        }
        let loads = self.load_extremes().expect("multiple LP schedulers have no load summaries");
        let (destination_lp, destination_load) = loads.least;
        let (source_lp, source_load) = loads.most;
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

    fn load_extremes(&self) -> Option<LoadExtremes> {
        LoadExtremes::sample(self.lp_schedulers.keys().map(|&lp| {
            assert!((lp as usize) < MAX_TRACKED_LPS);
            (lp, LP_LOAD_SUMMARIES[lp as usize].load(Ordering::Acquire))
        }))
    }

    /// Block the specified thread at least until the given event notifies its observers
    pub fn block_thread(
        &self,
        tid: ThreadId,
        event: &dyn crate::klib::observer::Observable,
    ) -> Result<(), Error> {
        self.block_thread_with_constraint_generation(tid, event, MigrationConstraint::GeneralWait)
            .map(|_| ())
    }

    pub fn block_thread_with_constraint(
        &self,
        tid: ThreadId,
        event: &dyn crate::klib::observer::Observable,
        constraint: MigrationConstraint,
    ) -> Result<(), Error> {
        self.block_thread_with_constraint_generation(tid, event, constraint).map(|_| ())
    }

    pub fn abort_thread(&self, tid: ThreadId) -> Result<ThreadId, Error> {
        let generation =
            MASTER_THREAD_TABLE.read().get(tid).map_err(|_| Error::InvalidThread)?.generation;
        self.abort_thread_generation(tid, generation)
    }

    /// Request only this LP's exact executing lifetime. The caller retains its
    /// local mask through completion of any owner held on the outgoing stack.
    /// Unlike general abort, this cannot scan peers, stage a context, send an
    /// IPI or perform retirement. LP authority precedes the thread-table check.
    fn request_executing_abort(
        &self,
        tid: ThreadId,
        generation: ThreadGeneration,
        root: crate::memory::AddressSpaceHandle,
        _handoff: &crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask,
    ) -> Result<(), Error> {
        let lp = get_lp_id();
        let local = self.get_lp_scheduler().lock();
        if local.get_current_handle() != Some((tid, generation)) {
            return Err(Error::InvalidThread);
        }
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(tid).map_err(|_| Error::InvalidThread)?;
        if thread.generation != generation || thread.address_space != Some(root) {
            return Err(Error::InvalidThread);
        }
        thread.abort_owner_lp.store(lp as usize, Ordering::Release);
        thread.abort_requested.store(true, Ordering::Release);
        local.set_ctx_switch_pending();
        Ok(())
    }

    /// Abort exactly one published thread lifetime.
    ///
    /// Numeric TIDs are recycled. Delayed cleanup code must use this method so
    /// it cannot kill a later occupant of the same table slot.
    pub fn abort_thread_generation(
        &self,
        tid: ThreadId,
        expected_generation: ThreadGeneration,
    ) -> Result<ThreadId, Error> {
        // Determine where the thread is known to be under a short-lived read
        // lock, so we do NOT hold a MASTER_THREAD_TABLE guard across the later
        // write lock (doing so would self-deadlock the non-reentrant RwLock).
        let state_lp = {
            let table = MASTER_THREAD_TABLE.read();
            match table.get(tid) {
                Ok(thread) if thread.generation == expected_generation => {
                    let executor = thread.operation_executor_lp.load(Ordering::Acquire);
                    if executor != usize::MAX {
                        // Record the request without extracting an off-CPU/blocked
                        // executor. Its owner completes the root before this fence.
                        thread.abort_owner_lp.store(executor, Ordering::Release);
                        thread.abort_requested.store(true, Ordering::Release);
                        return Ok(tid);
                    }
                    match thread.state {
                        ThreadState::Running(lp_id) | ThreadState::Ready(lp_id) => Some(lp_id),
                        _ => None,
                    }
                }
                Ok(_) | Err(_) => return Err(Error::InvalidThread),
            }
        };
        // The LP scheduler is the authority for a currently executing thread.
        // A migration/context-switch boundary can leave the separately stored
        // ThreadState snapshot briefly pointing at the previous LP; preferring
        // that stale snapshot makes self-abort remove from the wrong scheduler.
        let executing_lp = self.current_lp_for_thread_generation(tid, expected_generation);
        let current_lp = executing_lp.or(state_lp);
        // Keep an executing context and its scheduler handle until the owner
        // switches away, including self-abort. Removing the local handle here
        // makes the switch treat the outgoing context as absent, skipping its
        // save and ARM's on_cpu release handshake. Remote queued work also
        // remains owned until that LP's safe retirement boundary.
        if let Some(owner_lp) = current_lp
            && (owner_lp != get_lp_id() || executing_lp.is_some())
        {
            let table = MASTER_THREAD_TABLE.read();
            let thread = table.get(tid).map_err(|_| Error::InvalidThread)?;
            if thread.generation != expected_generation {
                return Err(Error::InvalidThread);
            }
            // Requalify the notification target under the same captured
            // generation guard as the request. A prior Ready-LP snapshot can
            // precede a complete queue move; the old LP cannot unlink it.
            let owner_lp = thread.abort_notification_lp(owner_lp);
            thread.abort_owner_lp.store(owner_lp as usize, Ordering::Release);
            thread.abort_requested.store(true, Ordering::Release);
            drop(table);
            if owner_lp == get_lp_id() {
                self.lp_schedulers[&owner_lp].lock().set_ctx_switch_pending();
            } else if LocalIntCtlr::send_unicast_ipi(owner_lp, SCHEDULER_IPI_VECTOR).is_err() {
                // Keep the request authoritative even if the prompt IPI could
                // not be delivered. The owner LP's periodic scheduler tick
                // will observe it and retire the thread; clearing it here
                // would let domain teardown wait forever on a live sibling.
                crate::early_logln!(
                    "WARNING: scheduler IPI to LP{} failed; abort of thread {} remains pending",
                    owner_lp,
                    tid
                );
            }
            return Ok(tid);
        }
        // Serialize final scheduler removal with executor admission. A target
        // can have resumed since the earlier snapshot; its exact LP handle and
        // inline owner must be rechecked before either queue/table mutation.
        let remove_lp = current_lp.or(state_lp);
        let mut local = remove_lp.map(|lp| self.lp_schedulers[&lp].lock());
        let _retirement = begin_retirement();
        let mut table = MASTER_THREAD_TABLE.write();
        let thread = table.get(tid).map_err(|_| Error::InvalidThread)?;
        if thread.generation != expected_generation {
            return Err(Error::InvalidThread);
        }
        let executor = thread.operation_executor_lp.load(Ordering::Acquire);
        if executor != usize::MAX
            || local
                .as_ref()
                .is_some_and(|lp| lp.get_current_handle() == Some((tid, expected_generation)))
        {
            let owner = if executor != usize::MAX {
                executor
            } else {
                remove_lp.unwrap() as usize
            };
            thread.abort_owner_lp.store(owner, Ordering::Release);
            thread.abort_requested.store(true, Ordering::Release);
            if let Some(lp) = local.as_ref() {
                lp.set_ctx_switch_pending();
            }
            return Ok(tid);
        }
        if let Some(lp) = local.as_mut() {
            lp.remove_thread(tid, Some(expected_generation), &mut table)
                .map_err(|_| Error::InvalidThread)?;
        }
        let stage_lp = current_lp.unwrap_or_else(get_lp_id);
        let thread = table.take_element(tid).map_err(|_| Error::InvalidThread)?;
        crate::cpu::scheduler::threads::account_retired_cpu_ticks(&thread);
        drop(table);
        drop(local);
        record_exit(stage_lp, tid, thread.generation);
        crate::cpu::scheduler::threads::stage_dead_thread(stage_lp, tid, thread);
        Ok(tid)
    }

    fn get_least_loaded_lp(&self) -> &Mutex<Box<dyn LpScheduler>> {
        self.lp_schedulers.iter().min_by_key(|sched| sched.1.lock().thread_count()).unwrap().1
    }

    fn current_lp_for_thread_generation(
        &self,
        tid: ThreadId,
        generation: ThreadGeneration,
    ) -> Option<LpId> {
        self.lp_schedulers.iter().find_map(|(&lp_id, sched)| {
            let scheduler = sched.lock();
            (scheduler.get_current_handle() == Some((tid, generation))).then_some(lp_id)
        })
    }
}

impl Default for SystemScheduler {
    fn default() -> Self {
        Self::new()
    }
}

pub fn get_thread_id() -> Option<ThreadId> {
    SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid()
}
