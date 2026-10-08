//! Thread control block, state machine, and the global thread table.
//!
//! A [`Thread`] represents a kernel-scheduled execution context.  Threads
//! transition through four states:
//!
//! ```text
//! NeedsLpAssignment → Ready(lp) → Running(lp) → Blocked(waker) → Ready(lp)
//!                         ↑                                      │
//!                         └──────────────────────────────────────┘
//! ```
//!
//! - `NeedsLpAssignment`: freshly spawned, not yet assigned to an LP.
//! - `Ready(lp)`: enqueued in LP `lp`'s run queue, waiting to run.
//! - `Running(lp)`: `current_handle` of LP `lp`'s scheduler; actively executing.
//! - `Blocked(waker)`: parked, waiting for an observable event to fire the waker.
//! - remote abort: mark `abort_requested`, interrupt the owning LP, switch the target off-CPU, and
//!   retire it from that LP's safe scheduling boundary.
//!
//! The [`MASTER_THREAD_TABLE`] is the system-wide [`IdTable`] keyed by
//! reusable [`ThreadId`].  Each entry is guarded by a monotonic
//! [`ThreadGeneration`] that prevents stale-handle-after-slot-reuse races.
//! Exited threads are staged per-LP in [`DEAD_THREADS`] and reaped after
//! the context switch away from them.

pub(crate) mod exit_source;
pub(crate) mod retirement_tests;
pub mod waker;

use alloc::{
    boxed::Box,
    sync::{
        Arc,
        Weak,
    },
    vec::Vec,
};
use core::{
    mem::offset_of,
    sync::atomic::{
        AtomicBool,
        AtomicU64,
        AtomicUsize,
        Ordering,
    },
};

use spin::LazyLock;

use crate::{
    cpu::{
        isa::lp::{
            LpId,
            thread_context::ThreadContext,
        },
        multiprocessor::spin::rwlock::RwLock,
        scheduler::threads::waker::Waker,
    },
    klib::{
        collections::{
            id_table::IdTable,
            retirement_list::{
                PreparedEntry,
                RetirementList,
            },
        },
        observer::Observer,
        statistics::{
            RunningStatistics,
            StatisticsSnapshot,
        },
    },
    memory::{
        AddressSpaceId,
        KERNEL_ASID,
    },
};

pub static MASTER_THREAD_TABLE: LazyLock<RwLock<ThreadTable>> =
    LazyLock::new(|| RwLock::new(ThreadTable::new()));
pub type ThreadTable = IdTable<Thread>;
pub type ThreadId = usize;
pub type ThreadGeneration = u64;

static NEXT_THREAD_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Runtime retained after a thread leaves [`MASTER_THREAD_TABLE`]. Keeping the
/// retired contribution makes the node-wide busy counter monotonic, which is
/// required for interval load samples in userspace.
static RETIRED_CPU_BUSY_TICKS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn account_retired_cpu_ticks(thread: &Thread) {
    let now = crate::cpu::scheduler::monotonic_ticks();
    let active = thread.last_dispatch_tick.map_or(0, |started| now.saturating_sub(started));
    let ticks = thread.runtime_ticks.snapshot().total.saturating_add(u128::from(active));
    let ticks = u64::try_from(ticks).unwrap_or(u64::MAX);
    let _ = RETIRED_CPU_BUSY_TICKS.try_update(Ordering::Relaxed, Ordering::Relaxed, |total| {
        Some(total.saturating_add(ticks))
    });
}

/// Threads that have exited but are awaiting reaping, keyed by the logical
/// processor on which they last executed. A thread cannot free its own kernel
/// stack during explicit pair release while it is still executing on it, so
/// `abort` stages the dying thread here instead of dropping it.
///
/// The list is **per-LP** on purpose. Both architectures use a scheduled
/// pinned reaper on the dying thread's LP, after switching away. Both retain
/// the node if its stack contains the executing SP; Arm additionally checks
/// the context's assembly ownership flag. A shared, cross-LP reaper could free
/// a stack before its owning LP has switched away.
static DEAD_THREADS: RwLock<
    [RetirementList<Thread>; crate::cpu::scheduler::system_scheduler::MAX_TRACKED_LPS],
> = RwLock::new(
    [const { RetirementList::new() }; crate::cpu::scheduler::system_scheduler::MAX_TRACKED_LPS],
);

pub(crate) fn has_staged_asid(asid: AddressSpaceId) -> bool {
    DEAD_THREADS.read().iter().flat_map(RetirementList::iter).any(|thread| thread.asid == asid)
}

pub(crate) fn has_staged_generation(generation: ThreadGeneration) -> bool {
    DEAD_THREADS
        .read()
        .iter()
        .flat_map(RetirementList::iter)
        .any(|thread| thread.generation == generation)
}

/// Number of threads currently moving from the master table into an LP's
/// deferred-reaping list.
///
/// The two collections have separate locks. Without this transition marker,
/// a lifecycle observer can see the thread in neither collection between the
/// remove and insert operations and incorrectly conclude that its domain has
/// exited.
static RETIREMENTS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static RETIREMENT_EPOCH: AtomicU64 = AtomicU64::new(0);

pub struct RetirementGuard;

impl Drop for RetirementGuard {
    fn drop(&mut self) {
        RETIREMENT_EPOCH.fetch_add(1, Ordering::SeqCst);
        let previous = RETIREMENTS_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(previous > 0, "thread retirement counter underflow");
    }
}

pub fn begin_retirement() -> RetirementGuard {
    RETIREMENTS_IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    RETIREMENT_EPOCH.fetch_add(1, Ordering::SeqCst);
    RetirementGuard
}

pub fn retirement_in_flight() -> bool {
    RETIREMENTS_IN_FLIGHT.load(Ordering::SeqCst) != 0
}

pub fn retirement_epoch() -> u64 {
    RETIREMENT_EPOCH.load(Ordering::SeqCst)
}

/// Stage a thread that has stopped being scheduled on `lp` for reaping by that
/// same LP. The thread's stack is not freed until [`reap_dead_threads`] runs on
/// `lp` after a context switch away from it.
pub fn stage_dead_thread(lp: LpId, tid: ThreadId, mut thread: Thread) {
    thread.retired_tid = Some(tid);
    thread.reap_lp = Some(lp);
    thread.trace_lifecycle(crate::debug_trace::THREAD_LIFECYCLE_STAGE, current_stack_pointer());
    let storage = thread.retirement.take().expect("thread retirement storage missing");
    DEAD_THREADS.write()[lp as usize].push(storage.publish(thread));
}

/// Complete abort requests after the owning LP has selected another
/// context. The old thread intentionally remains `Running(lp)` in the master
/// table during the switch so its saved-stack slot stays valid.
pub fn retire_requested_threads() {
    let lp = crate::cpu::isa::lp::ops::get_lp_id();
    let active_tid = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
        .read()
        .get_lp_scheduler()
        .lock()
        .get_tid();
    // Capture a finite scan ceiling, not a heap-backed teardown snapshot.
    // Each selected occupant is claimed under the table using its own request
    // and generation; new slots beyond the ceiling wait for the next boundary.
    let ceiling = MASTER_THREAD_TABLE.read().iter().len();
    for tid in 0..ceiling {
        let (thread, generation, _retirement) = {
            let mut table = MASTER_THREAD_TABLE.write();
            let requested = table.get(tid).is_ok_and(|thread| {
                thread.abort_owner_lp.load(Ordering::Acquire) == lp as usize
                    && Some(tid) != active_tid
                    && thread.abort_requested.load(Ordering::Acquire)
            });
            if !requested {
                continue;
            }
            let retirement = begin_retirement();
            let thread = table.take_element(tid).expect("validated abort target disappeared");
            let generation = thread.generation;
            account_retired_cpu_ticks(&thread);
            (thread, generation, retirement)
        };
        record_exit(lp, tid, generation);
        stage_dead_thread(lp, tid, thread);
    }
}

/// Install one pinned worker per LP for explicit stack-pair retirement.
pub(crate) fn start_reapers() {
    extern "C" fn reap() {
        loop {
            reap_dead_threads();
            crate::cpu::scheduler::sleep_millis(1);
        }
    }
    // A switch to this same-LP worker proves the dying context is off-CPU.
    // Workers stay pinned and blocked between batches; all physical cleanup
    // runs in guard-free thread context with IRQ/IPI progress available.
    for lp in 0..crate::cpu::multiprocessor::get_lp_count() {
        crate::cpu::scheduler::spawn_thread_on_lp(KERNEL_ASID, reap, lp);
    }
}

/// Must run after switching away from the dying thread, on its original LP.
/// Both architectures use scheduled workers for IRQ-enabled invalidation.
/// A masked caller rejects before claiming any retirement node.
pub fn reap_dead_threads() {
    if !crate::cpu::isa::lp::ops::get_int_state() {
        return;
    }
    let lp = crate::cpu::isa::lp::ops::get_lp_id();
    reap_dead_threads_with(lp, current_stack_pointer());
}

/// Own both the detached nodes and their lifecycle publication fence. Drop
/// retains the marker as well as nodes; abandonment cannot look like reaping.
struct ReapBatch {
    threads: RetirementList<Thread>,
    deferred: RetirementList<Thread>,
    transition: Option<RetirementGuard>,
}

impl ReapBatch {
    fn complete(mut self) {
        assert!(
            self.threads.is_empty() && self.deferred.is_empty(),
            "unfinished thread reap batch"
        );
        drop(self.transition.take());
    }
}

impl Drop for ReapBatch {
    fn drop(&mut self) {
        if let Some(transition) = self.transition.take() {
            core::mem::forget(transition);
        }
    }
}

// Boot fixtures call this only for never-admitted contexts. Production enters
// through the current-LP, IRQ-qualified public wrapper above.
pub(in crate::cpu::scheduler) fn reap_dead_threads_with(lp: LpId, current_sp: usize) {
    reap_dead_threads_releasing_with(lp, current_sp, Thread::retire_stacks);
}

fn reap_dead_threads_releasing_with(
    lp: LpId,
    current_sp: usize,
    mut release: impl FnMut(&mut Thread) -> Result<(), crate::memory::thread_stack::RetirementError>,
) {
    let mut batch = {
        let mut guard = DEAD_THREADS.write();
        let threads = &mut guard[lp as usize];
        if threads.is_empty() {
            return;
        }
        let transition = begin_retirement();
        ReapBatch {
            threads: core::mem::take(threads),
            deferred: RetirementList::new(),
            transition: Some(transition),
        }
    };
    // Staging prepends nodes. Preserve prior insertion-order notification while
    // walking the detached batch iteratively, outside the registry guard.
    batch.threads.reverse();
    while let Some(mut entry) = batch.threads.pop() {
        let thread = entry.value();
        // switch_ctx returns through the incoming context's older yield call.
        // Retain the actual executing stack, even on its owning LP. ARM also
        // supplies the assembly ownership handshake; x86 migration stays off.
        let in_use = thread.context.kernel_stack_contains(current_sp) || thread.context.is_on_cpu();
        let retained = in_use || thread.context.stack_retirement_started();
        let phase = if retained {
            crate::debug_trace::THREAD_LIFECYCLE_REAP_DEFER
        } else {
            crate::debug_trace::THREAD_LIFECYCLE_REAP_RECLAIM
        };
        thread.trace_lifecycle(phase, current_sp);
        if retained {
            batch.deferred.push(entry);
        } else if release(entry.value_mut()).is_ok() {
            entry.release();
        } else {
            // The same admitted node retains the entire failed pair. Its phase
            // fence forbids retry; retained backing is not a recovery receipt.
            batch.deferred.push(entry);
        }
    }
    batch.deferred.reverse();
    {
        let mut guard = DEAD_THREADS.write();
        while let Some(entry) = batch.deferred.pop() {
            guard[lp as usize].push(entry);
        }
    }
    batch.complete();
}

#[cfg(target_arch = "aarch64")]
pub(in crate::cpu::scheduler) fn current_stack_pointer() -> usize {
    let sp: usize;
    unsafe {
        core::arch::asm!("mov {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    sp
}

#[cfg(target_arch = "x86_64")]
pub(in crate::cpu::scheduler) fn current_stack_pointer() -> usize {
    let sp: usize;
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    sp
}

pub type ThreadCount = usize;

pub const SCHEDULER_DIAGNOSTIC_LPS: usize = 256;
pub const SCHEDULER_DIAGNOSTIC_FIELDS: usize = 6;
/// Per LP: current TID, generation, ASID, dispatch count, last exiting TID,
/// last exiting generation. Kept lock-free for debugger inspection.
#[unsafe(no_mangle)]
pub static SCHEDULER_LP_DIAGNOSTICS: [AtomicU64;
    SCHEDULER_DIAGNOSTIC_LPS * SCHEDULER_DIAGNOSTIC_FIELDS] =
    [const { AtomicU64::new(u64::MAX) }; SCHEDULER_DIAGNOSTIC_LPS * SCHEDULER_DIAGNOSTIC_FIELDS];

pub fn record_dispatch(
    lp: LpId,
    tid: ThreadId,
    generation: ThreadGeneration,
    asid: AddressSpaceId,
) {
    let base = lp as usize * SCHEDULER_DIAGNOSTIC_FIELDS;
    SCHEDULER_LP_DIAGNOSTICS[base].store(tid as u64, Ordering::Relaxed);
    SCHEDULER_LP_DIAGNOSTICS[base + 1].store(generation, Ordering::Relaxed);
    SCHEDULER_LP_DIAGNOSTICS[base + 2].store(asid as u64, Ordering::Relaxed);
    SCHEDULER_LP_DIAGNOSTICS[base + 3].fetch_add(1, Ordering::Relaxed);
}

pub fn record_exit(lp: LpId, tid: ThreadId, generation: ThreadGeneration) {
    let base = lp as usize * SCHEDULER_DIAGNOSTIC_FIELDS;
    SCHEDULER_LP_DIAGNOSTICS[base + 4].store(tid as u64, Ordering::Relaxed);
    SCHEDULER_LP_DIAGNOSTICS[base + 5].store(generation, Ordering::Relaxed);
}

/// Ownership relationships that prevent moving an otherwise Ready thread.
#[derive(Debug, Clone, Copy)]
pub enum MigrationConstraint {
    GeneralWait = 1 << 0,
    TimerWait = 1 << 1,
    CompletionQueueWait = 1 << 2,
    EndpointWait = 1 << 3,
    DeviceBound = 1 << 4,
    DeferredWork = 1 << 5,
}

impl MigrationConstraint {
    pub const fn bit(self) -> u32 {
        self as u32
    }
}

#[derive(Debug)]
pub enum ThreadState {
    Running(LpId),
    Ready(LpId),
    NeedsLpAssignment,
    Blocked(Arc<Waker>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ThreadStateKind {
    Running = 1,
    Ready = 2,
    NeedsLpAssignment = 3,
    Blocked = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadStatisticsSnapshot {
    pub tid: ThreadId,
    pub generation: ThreadGeneration,
    pub asid: AddressSpaceId,
    pub state: ThreadStateKind,
    pub affinity_lp: Option<LpId>,
    pub pinned_lp: Option<LpId>,
    pub dispatch_count: u64,
    pub runtime_ticks: StatisticsSnapshot,
    pub current_slice_started_at: Option<u64>,
    pub stack_reserved_pages: u64,
    pub stack_committed_pages: u64,
    pub stack_used_pages: u64,
}

#[derive(Debug)]
pub struct Thread {
    /// Boxed so the context (and therefore its `saved_sp`/`rsp_cpl0` field) has
    /// a **stable heap address**. `cond_yield_lp` captures a raw pointer to that
    /// field under lock and dereferences it lock-free inside `switch_ctx`; if the
    /// context lived inline in `MASTER_THREAD_TABLE`'s backing `Vec`, a
    /// concurrent `spawn_thread` on another LP that grows the `Vec` would move
    /// every `Thread` and leave that pointer dangling — corrupting the saved
    /// stack pointer of the thread being switched. The `Box` keeps the context
    /// pinned regardless of table reallocation.
    pub context: Box<ThreadContext>,
    pub asid: AddressSpaceId,
    pub(crate) address_space: Option<crate::memory::AddressSpaceHandle>,
    /// Distinguishes successive occupants of a reusable [`ThreadId`] slot.
    pub generation: ThreadGeneration,
    pub(crate) wait_sponsor: crate::klib::observer::WaitSponsor,
    pub(crate) timer_sponsor: crate::timers::budget::SchedulerSponsor,
    pub state: ThreadState,
    /// The LP this thread prefers to run on, assigned at spawn time.
    /// Re-admission via `submit_woken_thread` and initial `submit_new_thread`
    /// try this LP first rather than scanning for the least-loaded one,
    /// giving the thread cache affinity and keeping its timer events on
    /// the same LP's queue.
    pub affinity_lp: Option<LpId>,
    /// A hard placement constraint for work whose semantics are LP-local
    /// (notably shard workers). Unlike soft affinity, rebalancing must never
    /// change this value.
    pub pinned_lp: Option<LpId>,
    /// Explicit permission for Ready-state load migration. Hard pinning still
    /// takes precedence. Set false for work with unmodelled LP-local state.
    pub migration_safe: bool,
    /// Active temporary or permanent reasons why the thread must remain local.
    pub migration_constraints: u32,
    /// Completed on-CPU intervals, measured in raw architectural counter ticks.
    pub runtime_ticks: RunningStatistics,
    pub dispatch_count: u64,
    pub last_dispatch_tick: Option<u64>,
    /// Cross-LP termination is completed by the CPU that owns the running
    /// context, after it has switched off this thread's stack.
    pub(crate) abort_requested: AtomicBool,
    pub(crate) abort_owner_lp: AtomicUsize,
    /// Identity retained after removal from the master table for diagnostic
    /// correlation with the deferred-reaping and stack-deallocation paths.
    retired_tid: Option<ThreadId>,
    reap_lp: Option<LpId>,
    exit_observers: exit_source::ExitSource,
    // One empty node follows this owner until it stages itself. A staged
    // thread has None here: its containing node owns the whole payload.
    retirement: Option<PreparedEntry<Thread>>,
    retirement_metadata_completed: bool,
}

pub const THREAD_CTX_OFFSET: usize = offset_of!(Thread, context);

impl Thread {
    pub fn new(asid: AddressSpaceId, entry_point: extern "C" fn()) -> Self {
        Self::try_new(asid, entry_point).expect("mandatory thread construction failed")
    }

    pub(crate) fn try_new(
        asid: AddressSpaceId,
        entry_point: extern "C" fn(),
    ) -> Result<Self, crate::cpu::scheduler::system_scheduler::Error> {
        Self::try_new_with_retirement(asid, entry_point, PreparedEntry::try_new)
    }

    fn try_new_with_retirement(
        asid: AddressSpaceId,
        entry_point: extern "C" fn(),
        allocate: impl FnOnce() -> Result<PreparedEntry<Thread>, core::alloc::AllocError>,
    ) -> Result<Self, crate::cpu::scheduler::system_scheduler::Error> {
        Self::try_new_with_storage(
            asid,
            entry_point,
            allocate,
            Box::<ThreadContext>::try_new_uninit,
        )
    }

    fn try_new_with_storage(
        asid: AddressSpaceId,
        entry_point: extern "C" fn(),
        allocate: impl FnOnce() -> Result<PreparedEntry<Thread>, core::alloc::AllocError>,
        allocate_context: impl FnOnce() -> Result<
            Box<core::mem::MaybeUninit<ThreadContext>>,
            core::alloc::AllocError,
        >,
    ) -> Result<Self, crate::cpu::scheduler::system_scheduler::Error> {
        use crate::cpu::scheduler::system_scheduler::Error;
        let address_space = if asid == KERNEL_ASID {
            None
        } else {
            let handle =
                crate::memory::current_address_space_handle(asid).ok_or(Error::ThreadTerminated)?;
            if !crate::cpu::scheduler::system_scheduler::thread_admission_open(handle) {
                return Err(Error::ThreadTerminated);
            }
            Some(handle)
        };
        // Failure precedes generation claim, stack backing and publication.
        let retirement = allocate().map_err(|_| Error::ThreadPreparationFailed)?;
        let mut context_storage = allocate_context().map_err(|_| Error::ThreadPreparationFailed)?;
        let generation = NEXT_THREAD_GENERATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                charlotte_lifecycle::claim_generation(next).map(|(_, following)| following)
            })
            .map_err(|_| Error::ThreadPreparationFailed)?;
        let context = if asid != KERNEL_ASID {
            let limits = crate::memory::domain_limits(asid);
            ThreadContext::create_user_thread_context(
                address_space.expect("user identity captured"),
                entry_point,
                limits.user_stack_pages,
            )
            .map_err(|_| Error::ThreadPreparationFailed)?
        } else {
            ThreadContext::create_kernel_thread_context(entry_point)
                .map_err(|_| Error::ThreadPreparationFailed)?
        };
        context_storage.as_mut().write(context);
        // The whole context was initialized above; no fallible work follows
        // before its allocation becomes the stable owning Box.
        let context = unsafe { context_storage.assume_init() };
        if asid != KERNEL_ASID {
            crate::memory::usage::note_thread_created(
                asid,
                crate::memory::domain_limits(asid).user_stack_pages,
            );
        }
        Ok(Thread {
            context,
            asid,
            address_space,
            generation,
            wait_sponsor: crate::memory::budget::waiter_sponsor(asid),
            timer_sponsor: crate::memory::budget::timer_sponsor(asid),
            state: ThreadState::NeedsLpAssignment,
            affinity_lp: None,
            pinned_lp: None,
            migration_safe: false,
            migration_constraints: 0,
            runtime_ticks: RunningStatistics::new(),
            dispatch_count: 0,
            last_dispatch_tick: None,
            abort_requested: AtomicBool::new(false),
            abort_owner_lp: AtomicUsize::new(usize::MAX),
            retired_tid: None,
            reap_lp: None,
            exit_observers: exit_source::ExitSource::new(),
            retirement: Some(retirement),
            retirement_metadata_completed: false,
        })
    }

    fn trace_lifecycle(&self, phase: u64, current_sp: usize) {
        let (stack_base, stack_end) = self.context.kernel_stack_bounds();
        crate::debug_trace::trace_thread_lifecycle(crate::debug_trace::ThreadLifecycleEvent {
            phase,
            queue_lp: self.reap_lp.map_or(usize::MAX, |lp| lp as usize),
            tid: self.retired_tid.unwrap_or(usize::MAX),
            generation: self.generation,
            asid: self.asid,
            stack_base,
            stack_end,
            current_sp,
            on_cpu: u8::from(self.context.is_on_cpu()),
            abort_owner: self.abort_owner_lp.load(Ordering::Acquire),
        });
    }

    pub fn statistics_snapshot(&self, tid: ThreadId) -> ThreadStatisticsSnapshot {
        let state = match self.state {
            ThreadState::Running(_) => ThreadStateKind::Running,
            ThreadState::Ready(_) => ThreadStateKind::Ready,
            ThreadState::NeedsLpAssignment => ThreadStateKind::NeedsLpAssignment,
            ThreadState::Blocked(_) => ThreadStateKind::Blocked,
        };
        let (stack_reserved_pages, stack_used_pages) = self.context.user_stack_usage();
        let stack_committed_pages = self.context.user_stack_committed_pages();
        ThreadStatisticsSnapshot {
            tid,
            generation: self.generation,
            asid: self.asid,
            state,
            affinity_lp: self.affinity_lp,
            pinned_lp: self.pinned_lp,
            dispatch_count: self.dispatch_count,
            runtime_ticks: self.runtime_ticks.snapshot(),
            current_slice_started_at: self.last_dispatch_tick,
            stack_reserved_pages: stack_reserved_pages as u64,
            stack_committed_pages: stack_committed_pages as u64,
            stack_used_pages: stack_used_pages as u64,
        }
    }

    pub fn is_user_thread(&self) -> bool {
        self.asid != KERNEL_ASID
    }

    pub fn add_migration_constraint(&mut self, constraint: MigrationConstraint) {
        self.migration_constraints |= constraint.bit();
    }

    pub fn clear_blocking_migration_constraints(&mut self) {
        self.migration_constraints &= !(MigrationConstraint::GeneralWait.bit()
            | MigrationConstraint::TimerWait.bit()
            | MigrationConstraint::CompletionQueueWait.bit()
            | MigrationConstraint::EndpointWait.bit());
    }

    pub fn is_fully_migratable(&self) -> bool {
        self.migration_safe
            && self.pinned_lp.is_none()
            && self.migration_constraints == 0
            && !self.abort_requested.load(Ordering::Acquire)
            && !self.context.is_on_cpu()
    }
}

/// Extend the current user thread's stack to cover one faulting address.
///
/// Runs in the synchronous EL0 fault path, so it must not treat transient
/// contention as a failure: the scheduler and thread-table locks are the
/// interrupt-masking multiprocessor locks, whose owner always makes progress.
/// Returns the new committed low address when a page was mapped.
pub(crate) fn grow_current_user_stack(asid: AddressSpaceId, fault_addr: usize) -> Option<usize> {
    let (tid, generation) = {
        let scheduler = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER.read();
        let local = scheduler.get_lp_scheduler().lock();
        local.get_current_handle()?
    };
    let mut table = MASTER_THREAD_TABLE.write();
    let thread = table.get_mut(tid).ok()?;
    if thread.generation != generation || thread.asid != asid {
        return None;
    }
    thread.context.grow_user_stack(fault_addr)
}

/// Return owned snapshots for threads in one address space.
///
/// The filter is intentional: an eventual userspace export syscall can expose
/// the caller's own scheduler data without granting ambient visibility into
/// other protection domains.
pub fn statistics_for_asid(asid: AddressSpaceId) -> Vec<ThreadStatisticsSnapshot> {
    MASTER_THREAD_TABLE
        .read()
        .iter()
        .enumerate()
        .filter_map(|(tid, thread)| {
            thread
                .as_ref()
                .filter(|thread| thread.asid == asid)
                .map(|thread| thread.statistics_snapshot(tid))
        })
        .collect()
}

/// Return the number of scheduler-table entries currently owned by an address
/// space. This is intentionally a cheap diagnostic/accounting primitive; it
/// does not include threads that have already been removed from the table and
/// are waiting for deferred stack reaping.
pub fn thread_count_for_asid(asid: AddressSpaceId) -> usize {
    MASTER_THREAD_TABLE
        .read()
        .iter()
        .filter(|thread| thread.as_ref().is_some_and(|thread| thread.asid == asid))
        .count()
}

/// Cumulative on-CPU ticks across every thread on this node.
///
/// Retired contributions are retained so callers can derive interval
/// utilization by differencing this value and the monotonic counter.
pub(crate) fn cpu_busy_ticks() -> u128 {
    let now = crate::cpu::scheduler::monotonic_ticks();
    let live = MASTER_THREAD_TABLE.read().iter().filter_map(|thread| thread.as_ref()).fold(
        0u128,
        |sum, thread| {
            let active = thread.last_dispatch_tick.map_or(0, |started| now.saturating_sub(started));
            sum.saturating_add(thread.runtime_ticks.snapshot().total)
                .saturating_add(u128::from(active))
        },
    );
    u128::from(RETIRED_CPU_BUSY_TICKS.load(Ordering::Relaxed)).saturating_add(live)
}

/// Snapshot all scheduler-visible threads. Callers must enforce the
/// system-observer capability before invoking this function.
#[allow(dead_code)]
pub(crate) fn system_statistics() -> Vec<ThreadStatisticsSnapshot> {
    MASTER_THREAD_TABLE
        .read()
        .iter()
        .enumerate()
        .filter_map(|(tid, thread)| thread.as_ref().map(|thread| thread.statistics_snapshot(tid)))
        .collect()
}

impl Thread {
    pub(crate) fn try_observe_exit(
        &self,
        observer: Weak<dyn Observer>,
        charge: crate::completion::watch_budget::Charge,
    ) -> Result<exit_source::ExitRegistration, crate::klib::observer::registration::RegistrationError>
    {
        self.exit_observers.register(observer, charge)
    }

    pub(crate) fn exit_watch_count(&self) -> usize {
        self.exit_observers.registered()
    }
}

impl Thread {
    fn finish_retirement_metadata(&mut self) {
        if self.retirement_metadata_completed {
            return;
        }
        // Metadata and callbacks run once, while the stack's exact root lease
        // still prevents numeric-ASID reuse. General metadata Drop is separate.
        self.retirement_metadata_completed = true;
        if let ThreadState::Blocked(waker) = &self.state {
            waker.cancel_registration();
        }
        if self.asid != KERNEL_ASID {
            let (reserved, used) = self.context.user_stack_usage();
            crate::memory::usage::note_thread_released(self.asid, reserved, used);
        }
        self.exit_observers.notify_exit();
    }

    fn retire_stacks(&mut self) -> Result<(), crate::memory::thread_stack::RetirementError> {
        if self.context.stack_retirement_started() {
            return Err(crate::memory::thread_stack::RetirementError::AlreadyStarted);
        }
        self.finish_retirement_metadata();
        self.trace_lifecycle(
            crate::debug_trace::THREAD_LIFECYCLE_STACK_DEALLOCATE,
            current_stack_pointer(),
        );
        self.context.release_stacks()
    }

    /// Ordinary rejection of a context that was never scheduler-admitted.
    /// The caller must leave publication/scheduler/table guards before entry.
    #[allow(clippy::result_large_err)] // Returning the complete owner must not allocate.
    pub(crate) fn release_unstarted(mut self) -> Result<(), Self> {
        if !matches!(self.state, ThreadState::NeedsLpAssignment)
            || self.context.is_on_cpu()
            || self.context.kernel_stack_contains(current_stack_pointer())
            || self.retire_stacks().is_err()
        {
            return Err(self);
        }
        Ok(())
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        // No stack release here or in implicit context/stack field destruction.
        // Exit/wait metadata fallback remains a separate caller-context boundary.
        self.finish_retirement_metadata();
    }
}
