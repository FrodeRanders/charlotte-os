//! Event producer custody: unpublished submissions, typed weak observers and
//! timer cancellation. Installation/rollback use the original exact-record
//! checks; accepted worker/exit/timer events keep their existing completion
//! contract. Active fallback and outer guard/IRQ qualification are unchanged.
use super::*;

pub(crate) mod exit_tests;
pub(crate) mod timer_tests;

/// An [`Observer`] that completes a capability when the worker thread it is
/// registered against exits. This is the ABI's intended completion mechanism
/// (see `scheduler/threads/mod.rs:63-69`): a completion capability is registered
/// as an exit-observer of the thread performing the work, so the thread exiting
/// *is* the completion event.
struct CompletionExitObserver {
    asid: AddressSpaceId,
    cap: CompletionCap,
    /// The result to post when the thread exits.
    result: OpResult,
    completion: CompletionWeak,
}

impl Observer for CompletionExitObserver {
    fn notify(self: Arc<Self>) {
        if let Some(completion) = self.completion.upgrade() {
            let _ = complete_registered(self.asid, self.cap, completion, self.result.clone());
        }
    }
}

/// An [`Observer`] that completes a capability when a timer fires.
/// This is the mechanism backing `OpCode::Timer`: the timer event's
/// observer posts the terminal result, so a userspace caller that
/// blocks on `cq_wait_checked` is released at the deadline.
pub(super) struct CompletionTimerObserver {
    asid: AddressSpaceId,
    cap: CompletionCap,
    result: OpResult,
    completion: CompletionWeak,
}

impl CompletionTimerObserver {
    /// Captured identifier only; this borrow grants no capability ownership.
    pub(super) fn cap(&self) -> CompletionCap {
        self.cap
    }

    pub(super) fn new(
        asid: AddressSpaceId,
        cap: CompletionCap,
        result: OpResult,
        completion: CompletionWeak,
    ) -> Self {
        Self {
            asid,
            cap,
            result,
            completion,
        }
    }
}

impl Observer for CompletionTimerObserver {
    fn notify(self: Arc<Self>) {
        if let Some(completion) = self.completion.upgrade() {
            let _ = complete_registered(self.asid, self.cap, completion, self.result.clone());
        }
    }
}

/// A kernel-boundary owner for a completion's anonymous timer. Releasing a
/// record cancels its event, but the event retains its admission charge until
/// its owning LP actually removes the queue node.
pub(super) struct TimerCancellation(Option<crate::timers::TimerEventCancelHandle>);

impl TimerCancellation {
    pub(super) fn new(handle: crate::timers::TimerEventCancelHandle) -> Self {
        Self(Some(handle))
    }
}

impl Drop for TimerCancellation {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            let _ = crate::timers::cancel_event(handle).finish();
        }
    }
}

/// Own an unpublished event-watch submission and its entry reservation.
/// Rollback rechecks object identity, so teardown/reuse cannot revoke a new cap.
pub(crate) struct EventSubmission {
    asid: AddressSpaceId,
    cap: CompletionCap,
    completion: CompletionRef,
    charge: Option<watch_budget::Charge>,
    committed: bool,
}

impl EventSubmission {
    pub(crate) fn new(asid: AddressSpaceId) -> Result<Self, SubmitError> {
        let (cap, completion, charge) = submit_captured(asid, None, true, &OPERATION_IDS)?;
        Ok(Self {
            asid,
            cap,
            completion,
            charge,
            committed: false,
        })
    }

    pub(crate) fn cap(&self) -> CompletionCap {
        self.cap
    }

    pub(crate) fn completion(&self) -> &CompletionRef {
        &self.completion
    }

    pub(crate) fn take_charge(&mut self) -> watch_budget::Charge {
        self.charge.take().expect("event submission charge transferred twice")
    }

    pub(crate) fn install_watch_observation(
        &self,
        observer: Arc<dyn Observer>,
        registration: crate::klib::observer::registration::Registration<watch_budget::Charge>,
    ) -> Result<bool, SubmitError> {
        self.install_observation(observer, registration, false)
    }

    fn install_worker_observation(
        &self,
        observer: Arc<dyn Observer>,
        registration: crate::klib::observer::registration::Registration<watch_budget::Charge>,
    ) -> Result<(), SubmitError> {
        self.install_observation(observer, registration, true).map(|_| ())
    }

    /// Fence installation against namespace teardown/replacement, including
    /// when a kernel waiter retains the unpublished old completion object.
    pub(super) fn install_observation(
        &self,
        observer: Arc<dyn Observer>,
        registration: crate::klib::observer::registration::Registration<watch_budget::Charge>,
        worker: bool,
    ) -> Result<bool, SubmitError> {
        let installation = {
            let registry = COMPLETIONS.read();
            if !registry.get(&self.asid).is_some_and(|entries| {
                entries.table.get(&self.cap).is_some_and(|live| Arc::ptr_eq(live, &self.completion))
            }) {
                return Err(SubmitError::UnknownAddressSpace);
            }
            if worker {
                self.completion.install_worker_observation(observer, registration)
            } else {
                self.completion.set_event_observation(observer, registration)
            }
        };
        Ok(installation.finish())
    }

    pub(crate) fn commit(mut self) -> CompletionCap {
        self.committed = true;
        self.cap
    }
}

impl Drop for EventSubmission {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let retired = {
            let mut registry = COMPLETIONS.write();
            if let Some(entries) = registry.get_mut(&self.asid)
                && entries
                    .table
                    .get(&self.cap)
                    .is_some_and(|live| Arc::ptr_eq(live, &self.completion))
            {
                Some(capability_metadata::retire(entries, self.asid, self.cap).unwrap())
            } else {
                None
            }
        };
        self.completion.release_event_observation();
        drop(self.completion.detach_callbacks());
        if let Some(retired) = retired {
            retired.finish();
        }
    }
}

/// Roll back a capability-backed submission before it becomes externally
/// visible. Retain both metadata owners and remove any terminal backlog row
/// before returning its slot, then dispose outside the local registry hold.
pub(crate) fn abort_submission(asid: AddressSpaceId, cap: CompletionCap) -> Result<(), CapError> {
    let retired = {
        let mut registry = COMPLETIONS.write();
        let entries = registry.get_mut(&asid).ok_or(CapError::UnknownAddressSpace)?;
        capability_metadata::retire(entries, asid, cap)?
    };
    retired.completion().release_event_observation();
    drop(retired.completion().detach_callbacks());
    retired.finish();
    Ok(())
}

/// Submits a timer operation: creates a capability that auto-completes after
/// `timeout_ms` milliseconds. The returned cap delivers a completion ring entry
/// when the deadline expires, so a user-space service waiting on `cq_wait_checked` is
/// released exactly at the deadline.
pub fn submit_timer(asid: AddressSpaceId, timeout_ms: u64) -> Result<CompletionCap, SubmitError> {
    submit_timer_with_observer(asid, timeout_ms, &OPERATION_IDS, Arc::try_new)
}

pub(super) fn submit_timer_with_observer(
    asid: AddressSpaceId,
    timeout_ms: u64,
    ids: &operation_ids::OperationIds,
    allocate: impl FnOnce(
        CompletionTimerObserver,
    ) -> Result<Arc<CompletionTimerObserver>, core::alloc::AllocError>,
) -> Result<CompletionCap, SubmitError> {
    record::submit_timer_with_ids(asid, timeout_ms, ids, allocate)
}

/// Submits an operation that is performed by a freshly spawned kernel worker
/// thread, and returns a capability that completes **when the worker thread
/// exits**.
///
/// An exit observer completes the capability and wakes waiters when the
/// worker exits. The worker performs its work and returns without handling
/// the completion capability itself.
///
/// `result` is the terminal result posted when the worker exits.
pub fn submit_worker(
    asid: AddressSpaceId,
    worker_entry: extern "C" fn(),
    result: OpResult,
) -> Result<CompletionCap, SubmitError> {
    let mut submission = EventSubmission::new(asid)?;
    let completion = submission.completion().clone();
    let observer: Arc<dyn Observer> = Arc::try_new(CompletionExitObserver {
        asid,
        cap: submission.cap(),
        result,
        completion: Arc::downgrade(&completion),
    })
    .map_err(|_| SubmitError::WouldBlock)?;
    crate::cpu::scheduler::spawn_worker_after_observe(
        worker_entry,
        Arc::downgrade(&observer),
        submission.take_charge(),
        |registration| submission.install_worker_observation(observer, registration),
    )?;
    Ok(submission.commit())
}

/// Register a completion capability that fires when the EL0 thread `tid`
/// exits. A supplied spawn-time generation prevents a delayed userspace join
/// from attaching to a replacement in the same numeric TID slot.
///
/// Thread IDs are recycled, so a caller that captured a `ServiceDomain`
/// (which records the spawn-time generation) must pass that generation: if
/// the tid slot already holds a different thread, the observed thread is
/// guaranteed gone and the capability completes immediately instead of
/// joining a stranger that may never exit.
pub(crate) fn observe_thread_exit_with_generation(
    asid: AddressSpaceId,
    tid: crate::cpu::scheduler::threads::ThreadId,
    expected_generation: Option<crate::cpu::scheduler::threads::ThreadGeneration>,
) -> Result<CompletionCap, SubmitError> {
    observe_thread_exit_scoped(asid, tid, expected_generation, None)
}

/// EL0 adapter: target authority is restricted to the exact caller domain.
pub(crate) fn observe_own_thread_exit(
    asid: AddressSpaceId,
    tid: crate::cpu::scheduler::threads::ThreadId,
    expected_generation: Option<crate::cpu::scheduler::threads::ThreadGeneration>,
) -> Result<CompletionCap, SubmitError> {
    let handle =
        crate::memory::current_address_space_handle(asid).ok_or(SubmitError::WouldBlock)?;
    let operation = crate::memory::operation::AddressSpaceOperation::acquire(handle)
        .map_err(|_| SubmitError::WouldBlock)?;
    let result = observe_thread_exit_scoped(asid, tid, expected_generation, Some(handle));
    operation.release().map_err(|_| SubmitError::WouldBlock)?;
    result
}

fn observe_thread_exit_scoped(
    asid: AddressSpaceId,
    tid: crate::cpu::scheduler::threads::ThreadId,
    expected_generation: Option<crate::cpu::scheduler::threads::ThreadGeneration>,
    caller: Option<crate::memory::AddressSpaceHandle>,
) -> Result<CompletionCap, SubmitError> {
    let mut submission = EventSubmission::new(asid)?;
    let cap = submission.cap();
    let completion = submission.completion().clone();
    let observer: Arc<dyn Observer> = Arc::try_new(CompletionExitObserver {
        asid,
        cap,
        result: OpResult::Ok(0),
        completion: Arc::downgrade(&completion),
    })
    .map_err(|_| SubmitError::WouldBlock)?;
    let registration = if let Some(caller) = caller {
        crate::cpu::scheduler::observe_thread_exit_in_domain(
            caller,
            tid,
            expected_generation,
            Arc::downgrade(&observer),
            submission.take_charge(),
        )
    } else {
        match expected_generation {
            Some(generation) => crate::cpu::scheduler::observe_thread_exit_with_generation(
                tid,
                generation,
                Arc::downgrade(&observer),
                submission.take_charge(),
            ),
            None => crate::cpu::scheduler::observe_thread_exit(
                tid,
                Arc::downgrade(&observer),
                submission.take_charge(),
            ),
        }
    };
    match registration {
        Ok(registration) => {
            if submission.install_watch_observation(observer, registration)? {
                let _ = complete_registered(asid, cap, completion, OpResult::Cancelled);
            }
        }
        Err(crate::cpu::scheduler::system_scheduler::Error::InvalidThread) => {
            // The thread is gone; its exit already happened. Complete now so
            // the joiner observes a terminal state immediately.
            let _ = complete_registered(asid, cap, completion, OpResult::Ok(0));
        }
        Err(_) => return Err(SubmitError::WouldBlock),
    }
    Ok(submission.commit())
}
