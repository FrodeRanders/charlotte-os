//! Capability-backed record state, buffer and subscription custody.
//! Allocation, capability publication and callback preparation are children of
//! this owner. Registry/CQ publication still retains the complete terminal
//! receipt through local guard exit before disposal and captured-source wake.
use super::*;

pub(crate) mod callback_tests;
mod callbacks;
mod preparation;
mod submission;
mod terminal_tests;

use callbacks::observe_registered;
pub use callbacks::{
    CompletionObservation,
    ObserveError,
    observe,
};
pub(super) use submission::{
    submit as submit_with_ids,
    submit_timer as submit_timer_with_ids,
};

/// The lifecycle state of a submitted operation (architecture doc §12.1).
///
/// A [`Completion`] object exists only after a successful [`submit`] — i.e.
/// after the `Created → Submitted → Accepted` transitions have already
/// succeeded — so the modelled states begin at `InFlight`:
///
/// ```text
/// InFlight ──cancel──▶ CancelPending
///    │                     │
/// complete              complete (forced Cancelled)
///    ▼                     ▼
/// Completed ◀─────────────┘
///    │
///  take (drain result + buffer)
///    ▼
/// Observed
/// ```
///
/// `Completed` and `Observed` are the terminal, reclaimable states. This
/// replaces the previous scattered `cancelling`/`drained` booleans with named
/// states and explicit transitions (architecture doc §18.2).
enum OpState {
    /// Submitted, no terminal result yet.
    InFlight,
    /// Cancellation requested while in flight; the terminal result will be
    /// forced to [`OpResult::Cancelled`].
    CancelPending,
    /// A terminal result has been posted but not yet drained.
    Completed(OpResult),
    /// The terminal result has been drained by [`Completion::take`].
    Observed,
}

struct CompletionInner {
    buffer: Option<Vec<u8>>,
    /// The operation's lifecycle state; all transitions are made under the
    /// mutex through the methods below (see [`OpState`]).
    state: OpState,
    /// Worker cancellation remains deferred until its producer exits. Unlike
    /// an external join, cancelling it must not revoke the terminal event.
    worker_observation: Option<EventObservation>,
    /// Keeps the timer observer (if any) alive until the completion is reclaimed.
    timer_observer: Option<Arc<CompletionTimerObserver>>,
    timer_cancel: Option<TimerCancellation>,
    /// Keeps a subsystem-defined event observer alive. IPC uses this for
    /// connection endpoint-close watches without coupling completion storage
    /// to the concrete IPC observer type.
    event_observation: Option<EventObservation>,
    /// Non-scheduler callbacks use a separate lazy one-shot list. Its entries
    /// use event-watch admission independently of scheduler-waiter sponsorship.
    callbacks: Option<crate::klib::observer::registration::ListRef<watch_budget::Charge>>,
}

struct EventObservation {
    _observer: Arc<dyn Observer>,
    _registration: crate::klib::observer::registration::Registration<watch_budget::Charge>,
}

/// Installation returns every replaced or rejected owner to the caller. Its
/// explicit disposal must follow both inner and containing registry unlock.
#[must_use]
pub(crate) struct ObservationInstallation {
    cancellation_won: bool,
    unused: Option<EventObservation>,
}

impl ObservationInstallation {
    pub(crate) fn finish(self) -> bool {
        drop(self.unused);
        self.cancellation_won
    }
}

/// One terminal transition retains its detached producer subscriptions until
/// CQ publication and registry unlock. This is not a physical cleanup receipt.
pub(super) struct TerminalTransition {
    pub(super) result: OpResult,
    _event: Option<EventObservation>,
    _worker: Option<EventObservation>,
}

/// A kernel object naming one in-flight or completed operation.
///
/// It is [`Observable`]: threads awaiting the operation register their `Waker`
/// here (via [`wait`]), and [`Completion::complete`] notifies them — the exact
/// pattern `TimerEvent` uses for `sleep`.
pub struct Completion {
    /// Stable, never-reused identity of this operation (see [`OperationId`]).
    operation: OperationId,
    inner: Mutex<CompletionInner>,
    pub(super) waiters:
        crate::klib::observer::registration::ListRef<crate::klib::observer::waiter_budget::Charge>,
}

impl Completion {
    /// Assemble the worker subscription before taking the record guard. The
    /// containing submission retains installation/disposal ordering outside it.
    pub(super) fn install_worker_observation(
        &self,
        observer: Arc<dyn Observer>,
        registration: crate::klib::observer::registration::Registration<watch_budget::Charge>,
    ) -> ObservationInstallation {
        self.set_worker_observation(EventObservation {
            _observer: observer,
            _registration: registration,
        })
    }

    pub(super) fn new(
        operation: OperationId,
        buffer: Option<Vec<u8>>,
        record_charge: budget::Charge,
    ) -> Result<CompletionRef, SubmitError> {
        preparation::new(operation, buffer, record_charge)
    }

    pub(super) fn operation_id(&self) -> OperationId {
        self.operation
    }

    fn set_worker_observation(&self, observation: EventObservation) -> ObservationInstallation {
        let mut unused = Some(observation);
        let mut inner = self.inner.lock();
        if matches!(inner.state, OpState::InFlight | OpState::CancelPending) {
            core::mem::swap(&mut inner.worker_observation, &mut unused);
        }
        ObservationInstallation {
            cancellation_won: false,
            unused,
        }
    }

    pub(super) fn set_timer_observer(
        &self,
        observer: Arc<CompletionTimerObserver>,
        cancel: TimerCancellation,
    ) {
        let mut inner = self.inner.lock();
        inner.timer_observer = Some(observer);
        inner.timer_cancel = Some(cancel);
    }

    /// Returns whether cancellation won before this owner could be installed.
    /// A terminal completion must not retain a late registration.
    pub(crate) fn set_event_observation(
        &self,
        observer: Arc<dyn Observer>,
        registration: crate::klib::observer::registration::Registration<watch_budget::Charge>,
    ) -> ObservationInstallation {
        let mut unused = Some(EventObservation {
            _observer: observer,
            _registration: registration,
        });
        let mut inner = self.inner.lock();
        let cancellation_won = match inner.state {
            OpState::InFlight => {
                core::mem::swap(&mut inner.event_observation, &mut unused);
                false
            }
            OpState::CancelPending => true,
            OpState::Completed(_) | OpState::Observed => false,
        };
        ObservationInstallation {
            cancellation_won,
            unused,
        }
    }

    pub(super) fn release_event_observation(&self) {
        let observations = {
            let mut inner = self.inner.lock();
            (inner.event_observation.take(), inner.worker_observation.take())
        };
        drop(observations);
    }

    pub(super) fn state_kind(&self) -> OpStateKind {
        match self.inner.lock().state {
            OpState::InFlight => OpStateKind::InFlight,
            OpState::CancelPending => OpStateKind::CancelPending,
            OpState::Completed(_) => OpStateKind::Completed,
            OpState::Observed => OpStateKind::Observed,
        }
    }

    /// A terminal result has been posted (whether or not it has been drained).
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(self.inner.lock().state, OpState::Completed(_) | OpState::Observed)
    }

    pub(super) fn is_reclaimable(&self) -> bool {
        self.is_terminal()
    }

    pub(super) fn holds_buffer(&self) -> bool {
        self.inner.lock().buffer.is_some()
    }

    /// Kernel-side transition: the operation finished. Moves `InFlight` or
    /// `CancelPending` to `Completed`, forcing the result to
    /// [`OpResult::Cancelled`] when a cancel was requested. Returns the
    /// effective terminal result and detached subscriptions on the first call,
    /// or `None` if the operation
    /// was already terminal (idempotent). Does **not** signal observers; the
    /// caller wakes waiters after publishing the CQ entry.
    pub(super) fn complete(&self, result: OpResult) -> Option<TerminalTransition> {
        let mut inner = self.inner.lock();
        let effective = match inner.state {
            OpState::InFlight => result,
            OpState::CancelPending => OpResult::Cancelled,
            OpState::Completed(_) | OpState::Observed => return None,
        };
        inner.state = OpState::Completed(effective.clone());
        Some(TerminalTransition {
            result: effective,
            _event: inner.event_observation.take(),
            _worker: inner.worker_observation.take(),
        })
    }

    pub(super) fn signal(&self) {
        self.waiters.close().notify();
        self.detach_callbacks().notify();
    }

    pub(super) fn detach_callbacks(
        &self,
    ) -> crate::klib::observer::registration::NotificationBatch<watch_budget::Charge> {
        let callbacks = self.inner.lock().callbacks.take();
        callbacks
            .map_or_else(crate::klib::observer::registration::NotificationBatch::empty, |list| {
                list.close()
            })
    }

    /// Drains the terminal result and returns the buffer to the caller,
    /// transitioning `Completed → Observed`. Returns `None` while in flight or
    /// once already drained.
    pub(super) fn take(&self) -> Option<Completed> {
        let mut inner = self.inner.lock();
        match core::mem::replace(&mut inner.state, OpState::Observed) {
            OpState::Completed(result) => {
                let buffer = inner.buffer.take();
                Some(Completed {
                    result,
                    buffer,
                })
            }
            // Restore the prior state: nothing was drained.
            other => {
                inner.state = other;
                None
            }
        }
    }

    /// Requests cancellation. Transitions `InFlight → CancelPending`; if already
    /// terminal, reports it. The buffer is deliberately retained until a
    /// terminal completion is posted (deferred reclaim).
    pub(super) fn cancel(&self) -> CancelState {
        let mut inner = self.inner.lock();
        match inner.state {
            OpState::InFlight => {
                inner.state = OpState::CancelPending;
                CancelState::CancelRequested
            }
            OpState::CancelPending => CancelState::CancelRequested,
            OpState::Completed(_) | OpState::Observed => CancelState::AlreadyComplete,
        }
    }
}

impl Observable for Completion {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &crate::klib::observer::WaitSponsor,
    ) -> Result<
        crate::klib::observer::WaitRegistration,
        crate::klib::observer::registration::RegistrationError,
    > {
        if self.is_terminal() {
            return Ok(crate::klib::observer::WaitRegistration::ready());
        }
        match sponsor.register(&self.waiters, observer) {
            Err(crate::klib::observer::registration::RegistrationError::Closed)
                if self.is_terminal() =>
            {
                Ok(crate::klib::observer::WaitRegistration::ready())
            }
            result => result,
        }
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        // Retained tokens must not keep source entries after the operation is
        // destroyed. Destruction discards callbacks, never invents completion.
        drop(self.waiters.close());
        drop(self.detach_callbacks());
    }
}

/// Requests cancellation of an in-flight operation. See [`CancelState`]. Any
/// transferred buffer is retained until the terminal completion hands it back.
pub fn cancel(asid: AddressSpaceId, cap: CompletionCap) -> Result<CancelState, CapError> {
    let completion = completion_of(asid, cap)?;
    let state = completion.cancel();
    if state == CancelState::CancelRequested {
        let (timer, event) = {
            let mut inner = completion.inner.lock();
            (inner.timer_cancel.take(), inner.event_observation.take())
        };
        if timer.is_some() || event.is_some() {
            drop((timer, event));
            let _ = complete_registered(asid, cap, completion, OpResult::Cancelled);
        }
    }
    Ok(state)
}
