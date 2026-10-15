//! Per-LP scheduler trait and quantum timer observer.
//!
//! [`LpScheduler`] is the trait implemented by per-LP scheduling policies
//! (currently only [`RoundRobin`](crate::cpu::scheduler::lp_schedulers::round_robin::RoundRobin)).
//! It manages a run queue of ready threads, tracks the current thread, and
//! honours a `ctx_switch_pending` flag set by the quantum timer, IPI handlers,
//! and cooperative yield.
//!
//! [`TimerEventObserver`] tracks two atomic booleans — `pending` ("a context
//! switch has been requested") and `armed` ("a quantum TimerEvent is in
//! flight").  At most one quantum event is in flight per LP; the CAS on
//! `armed` prevents duplication on repeated `yield_lp` calls.

pub mod round_robin;
pub(crate) mod run_queue;
use alloc::{
    fmt::Debug,
    sync::Arc,
};
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use crate::{
    cpu::{
        isa::lp::LpId,
        scheduler::threads::{
            ThreadCount,
            ThreadGeneration,
            ThreadId,
            ThreadTable,
        },
    },
    klib::observer::Observer,
};

pub trait LpScheduler: Debug + Send {
    fn get_lp_id(&self) -> LpId;
    fn get_tid(&self) -> Option<ThreadId>;
    fn get_current_handle(&self) -> Option<(ThreadId, ThreadGeneration)>;
    fn is_ctx_switch_pending(&self) -> bool;
    /* The following two functions should use interior mutability to access an internal atomic
     * for safe lock free operation. */
    fn set_ctx_switch_pending(&self);
    /* This should clear the pending context switch field and when appropriate create and submit
     * a new TimerEvent to the local TimerQueue so the pending context switch will get set to
     * true again when the event notifies */
    fn clear_ctx_switch_pending(&self);
    fn next(&mut self) -> Result<ThreadId, Error>;
    fn add_thread(
        &mut self,
        tid: ThreadId,
        expected_generation: Option<ThreadGeneration>,
    ) -> Result<(), Error>;
    fn remove_thread(
        &mut self,
        tid: ThreadId,
        expected_generation: Option<ThreadGeneration>,
        table: &mut ThreadTable,
    ) -> Result<(), Error>;
    /// Find the first qualified queued lifetime, without allocating or
    /// acquiring the thread table again. The caller holds both LP locks and
    /// the supplied table through queue/state/affinity publication.
    fn ready_migration_candidate(
        &self,
        table: &ThreadTable,
    ) -> Result<Option<(ThreadId, ThreadGeneration)>, Error>;
    /// Complete a Ready-thread queue/state/affinity move. Callers hold both LP
    /// locks and the exact table. Rejection leaves both owners unchanged.
    fn migrate_ready_to(
        &mut self,
        destination: &mut dyn LpScheduler,
        table: &mut ThreadTable,
        tid: ThreadId,
        generation: ThreadGeneration,
    ) -> Result<(), Error> {
        let source_lp = self.get_lp_id();
        let destination_lp = destination.get_lp_id();
        migrate_ready(
            source_lp,
            self.ready_queue(),
            destination_lp,
            destination.ready_queue(),
            table,
            run_queue::ThreadHandle {
                tid,
                generation,
            },
        )?;
        self.refresh_load();
        destination.refresh_load();
        Ok(())
    }
    fn ready_queue(&mut self) -> &mut run_queue::RunQueue;
    fn refresh_load(&self);
    fn is_idle(&self) -> bool;
    fn start(&mut self);
    fn stop(&mut self);
    fn thread_count(&self) -> ThreadCount;
}

/// One complete queue/state publication under the caller's two LP guards and
/// exclusive table. All ordinary rejection precedes source mutation. Native
/// fixtures exercise this same operation without changing live LP summaries.
pub(crate) fn migrate_ready(
    source_lp: LpId,
    source: &mut run_queue::RunQueue,
    destination_lp: LpId,
    destination: &mut run_queue::RunQueue,
    table: &mut ThreadTable,
    handle: run_queue::ThreadHandle,
) -> Result<(), Error> {
    let thread = table.get(handle.tid).map_err(|_| Error::InvalidThread)?;
    if source_lp == destination_lp
        || thread.generation != handle.generation
        || !thread.is_fully_migratable()
        || !matches!(thread.state, crate::cpu::scheduler::threads::ThreadState::Ready(lp) if lp == source_lp)
    {
        return Err(Error::InvalidThread);
    }
    source.transfer_to(destination, table, handle).map_err(|_| Error::InvalidThread)?;
    let thread = table.get_mut(handle.tid).expect("validated migrating thread vanished");
    thread.state = crate::cpu::scheduler::threads::ThreadState::Ready(destination_lp);
    thread.affinity_lp = Some(destination_lp);
    Ok(())
}

#[derive(Debug)]
pub enum Error {
    NoRunnableThreads,
    ThreadAlreadyAssignedToLp,
    ThreadNotAssignedToThisLp,
    InvalidThread,
}

/// Tracks whether a per-LP context switch has been requested. The timer queue
/// owns quantum-event uniqueness; keeping a second `armed` state here can
/// strand runnable work if it ever diverges from the queue.
#[derive(Debug)]
struct TimerEventObserver {
    pending: AtomicBool,
}

impl TimerEventObserver {
    fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
        }
    }
}

impl Observer for TimerEventObserver {
    fn notify(self: Arc<Self>) {
        self.pending.store(true, Ordering::Release);
    }
}
