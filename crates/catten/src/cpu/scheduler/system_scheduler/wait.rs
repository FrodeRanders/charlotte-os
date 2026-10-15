//! Prepare subscriptions outside LP/thread-table guards, then qualify the
//! captured lifetime and publish Blocked without allocator work or callbacks.

use super::*;
use crate::klib::observer::{
    Observable,
    Observer,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Snapshot {
    Running(LpId),
    Ready(LpId),
    NeedsLpAssignment,
    Blocked,
}

impl Snapshot {
    fn of(state: &ThreadState) -> Self {
        match state {
            ThreadState::Running(lp) => Self::Running(*lp),
            ThreadState::Ready(lp) => Self::Ready(*lp),
            ThreadState::NeedsLpAssignment => Self::NeedsLpAssignment,
            ThreadState::Blocked(_) => Self::Blocked,
        }
    }
}

struct PreparedWait {
    tid: ThreadId,
    generation: ThreadGeneration,
    state: Snapshot,
    ready: bool,
    waker: Arc<waker::Waker>,
}

impl SystemScheduler {
    /// An early source hint is retained by the prepared waker. Under the table
    /// guard, either it prevents parking or its matching callback observes the
    /// published Blocked state. No source callback runs under these guards.
    pub fn block_thread_with_constraint_generation(
        &self,
        tid: ThreadId,
        event: &dyn Observable,
        constraint: MigrationConstraint,
    ) -> Result<ThreadGeneration, Error> {
        let prepared = self.prepare_wait(tid, event)?;
        let result = self.publish_wait(&prepared, constraint);
        if !matches!(result, Ok(true)) {
            // Immediate readiness, early notification and every rejection all
            // dispose the unused subscription after LP/table guards leave.
            prepared.waker.cancel_registration();
        }
        result.map(|_| prepared.generation)
    }

    fn prepare_wait(&self, tid: ThreadId, event: &dyn Observable) -> Result<PreparedWait, Error> {
        let (generation, state, sponsor) = {
            let table = MASTER_THREAD_TABLE.read();
            let thread = table.get(tid).map_err(|_| Error::InvalidThread)?;
            if matches!(thread.state, ThreadState::Blocked(_)) {
                return Err(Error::AlreadyBlocked);
            }
            if thread.abort_ready() {
                return Err(Error::ThreadTerminated);
            }
            (thread.generation, Snapshot::of(&thread.state), thread.wait_sponsor.clone())
        };
        let waker = Arc::try_new(waker::Waker::new(tid, generation))
            .map_err(|_| Error::WaitRegistrationFailed)?;
        let registration = event
            .try_register_waiter(
                Arc::downgrade(&waker) as alloc::sync::Weak<dyn Observer>,
                &sponsor,
            )
            .map_err(|_| Error::WaitRegistrationFailed)?;
        let ready = registration.is_ready();
        waker.set_registration(registration);
        Ok(PreparedWait {
            tid,
            generation,
            state,
            ready,
            waker,
        })
    }

    /// Borrows preparation: all losing storage and the caller's strong waker
    /// remain outside local guards. Success clones only the Arc into Thread.
    fn publish_wait(
        &self,
        prepared: &PreparedWait,
        constraint: MigrationConstraint,
    ) -> Result<bool, Error> {
        // Ready publication uses the canonical LP -> table order. A migrated
        // or otherwise changed snapshot rejects before either queue mutation.
        let mut lp_guard = match prepared.state {
            Snapshot::Ready(lp) => Some(self.lp_schedulers[&lp].lock()),
            _ => None,
        };
        let mut table = MASTER_THREAD_TABLE.write();
        let thread = table.get(prepared.tid).map_err(|_| Error::InvalidThread)?;
        if thread.generation != prepared.generation {
            return Err(Error::InvalidThread);
        }
        if matches!(thread.state, ThreadState::Blocked(_)) {
            return Err(Error::AlreadyBlocked);
        }
        if thread.abort_ready() {
            return Err(Error::ThreadTerminated);
        }
        if Snapshot::of(&thread.state) != prepared.state {
            return Err(Error::InvalidThread);
        }
        if prepared.ready || prepared.waker.was_notified() {
            return Ok(false);
        }
        if let Some(lp) = lp_guard.as_mut() {
            lp.remove_thread(prepared.tid, Some(prepared.generation), &mut table)
                .map_err(|_| Error::InvalidThread)?;
        }
        let thread = table.get_mut(prepared.tid).expect("validated waiting thread vanished");
        thread.add_migration_constraint(constraint);
        // A Running self-block keeps its current handle until the outgoing
        // context save; next() declines to requeue Blocked. Do not extract it.
        thread.state = ThreadState::Blocked(prepared.waker.clone());
        Ok(true)
    }
}

/// The closure contains every LP/table guard. Carry the removed wait owner
/// out even on a later admission error; cancellation may free heap metadata.
pub(super) fn with_detached_wait<T>(
    publish: impl FnOnce(&mut Option<Arc<waker::Waker>>) -> Result<T, Error>,
) -> Result<T, Error> {
    let mut detached = None;
    let result = publish(&mut detached);
    if let Some(waker) = detached {
        waker.cancel_registration();
    }
    result
}

mod tests;

pub(super) fn test_publication() {
    tests::run();
}
