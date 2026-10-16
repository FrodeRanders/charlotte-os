//! Per-LP sorted timer queue, backed by the ARM Generic Timer.
//!
//! Each LP has a [`TimerQueue`] — a sorted owning list of anonymous events and
//! one inline scheduler quantum. Their earliest deadline programs the hardware
//! comparator. A processing pass captures its due prefix and rearms the queue
//! before notifying outside its borrow on the same LP, with IRQ policy intact.
//!
//! Queue mutation reconciles the comparator with the earliest deadline.
//! Cancelled anonymous timers retain their admission charge until the owning
//! queue/delivery releases their actual backing. The idle loop calls `process_local_events`
//! before `wfi` to reconcile the software queue with the hardware comparator
//! and prevent missed deadlines after timer transitions.

pub(crate) mod budget;
mod cancellation_disposal;
mod cancellation_tests;
pub(crate) use cancellation_disposal::CancelledTimer;
mod delivery;
mod disposal_tests;
use delivery::PendingDelivery;
mod event_preparation;
pub(crate) mod event_tests;
mod node_preparation;
mod preparation_tests;
mod queue;
pub(crate) use queue::RetiredEvents;
pub(crate) mod waiter_tests;

use alloc::{
    boxed::Box,
    sync::{
        Arc,
        Weak,
    },
};
use core::{
    mem::{
        ManuallyDrop,
        MaybeUninit,
    },
    sync::atomic::{
        AtomicBool,
        AtomicU32,
        AtomicU64,
        Ordering,
    },
};

use spin::LazyLock;

use crate::{
    cpu::{
        isa::{
            interface::timers::{
                LpTimerError,
                LpTimerIfce,
            },
            lp::ops::{
                mask_interrupts,
                unmask_interrupts,
            },
            timers::LpTimer,
        },
        multiprocessor::spin::{
            mutex::Mutex,
            per_lp::PerLp,
        },
    },
    klib::{
        charged_allocator::ChargedAllocator,
        observer::{
            Observable,
            Observer,
            WaitRegistration,
            WaitSponsor,
            registration::RegistrationError,
            waiter_source::WaiterSource,
        },
        time::duration::ExtDuration,
    },
};

pub static TIMER_QUEUES: LazyLock<PerLp<TimerQueue>> =
    LazyLock::new(|| PerLp::new(TimerQueue::default));

const MAX_DIAGNOSTIC_LPS: usize = 256;

/// Low-perturbation timer lifecycle counters, readable from an external
/// debugger even when scheduler tracing is disabled. For anonymous events,
/// `added == fired + cancelled + queued` once the sampled queue is quiescent.
#[repr(C)]
pub struct TimerDiagnostic {
    pub anonymous_added: AtomicU64,
    pub anonymous_fired: AtomicU64,
    pub anonymous_cancelled: AtomicU64,
    pub keyed_added: AtomicU64,
    pub keyed_fired: AtomicU64,
    pub queue_len: AtomicU64,
    pub anonymous_queued: AtomicU64,
    pub front_deadline: AtomicU64,
    pub sampled_now: AtomicU64,
}

impl TimerDiagnostic {
    const fn new() -> Self {
        Self {
            anonymous_added: AtomicU64::new(0),
            anonymous_fired: AtomicU64::new(0),
            anonymous_cancelled: AtomicU64::new(0),
            keyed_added: AtomicU64::new(0),
            keyed_fired: AtomicU64::new(0),
            queue_len: AtomicU64::new(0),
            anonymous_queued: AtomicU64::new(0),
            front_deadline: AtomicU64::new(0),
            sampled_now: AtomicU64::new(0),
        }
    }
}

#[unsafe(no_mangle)]
pub static TIMER_DIAGNOSTICS: [TimerDiagnostic; MAX_DIAGNOSTIC_LPS] =
    [const { TimerDiagnostic::new() }; MAX_DIAGNOSTIC_LPS];

/// A capability to cancel one anonymous timer event.
///
/// Timer queues are LP-local because their head programs LP-local hardware.
/// The shared flag prevents notification even if a future scheduler change
/// resumes the waiter on a different LP; the owner removes the event eagerly
/// whenever cancellation happens on its original LP.
/// Abandonment retains this exact reference without flagging, cleanup or refund.
#[must_use = "cancel into an owned handback, or explicitly release this reference"]
pub(crate) struct TimerEventCancelHandle {
    state: ManuallyDrop<CancellationRef>,
}

impl TimerEventCancelHandle {
    /// Release only this reference after local guards leave. This does not
    /// cancel a queued/selected event or prove that its producer completed.
    pub(crate) fn release(self) {
        drop(ManuallyDrop::into_inner(self.state));
    }
}

// One event reservation covers its node and cancellation backing. Clones used
// as lifetime owners never allocate; only the cancellation Arc uses its single
// allocation allowance. The node carries an outside owner through Box release.
type EventAdmission = ChargedAllocator<budget::Charge>;
type CancellationRef = Arc<TimerEventCancellation, EventAdmission>;

pub type Timestamp = <LpTimer as LpTimerIfce>::Timestamp;

/// Owning, fallibly prepared anonymous queue node. Preparation allocates but
/// holds no queue guard or interrupt mask; callers mask park + publication.
/// Enqueue additionally masks its local comparator transaction and updates
/// shared cancellation ownership to the actual publishing LP.
/// Abandonment retains the complete node/source/admission without cleanup.
/// Ordinary unused preparation must be consumed by `cancel_unpublished` after
/// local publication/registry guards leave; outer caller contexts still apply.
#[must_use = "enqueue or explicitly cancel unused timer preparation"]
pub(crate) struct PreparedEvent {
    node: ManuallyDrop<queue::OwnedNode>,
}

impl PreparedEvent {
    pub(crate) fn new(event: TimerEvent) -> Result<Self, ()> {
        Self::new_with(event, || Box::<queue::Node>::try_new_uninit().map_err(|_| ()))
    }

    fn new_with(
        event: TimerEvent,
        allocate: impl FnOnce() -> Result<Box<MaybeUninit<queue::Node>>, ()>,
    ) -> Result<Self, ()> {
        node_preparation::prepare(event, allocate)
    }

    pub(crate) fn event(&self) -> &TimerEvent {
        &self.node.event
    }

    pub(crate) fn event_mut(&mut self) -> &mut TimerEvent {
        &mut self.node.event
    }

    fn into_node(self) -> queue::OwnedNode {
        ManuallyDrop::into_inner(self.node)
    }

    /// Dispose only this still-private node. This grants no queue cancellation,
    /// operation completion, physical release or retry authority.
    pub(crate) fn cancel_unpublished(self) {
        self.cancel_unpublished_with(|_| {});
    }

    fn cancel_unpublished_with(self, before_dispose: impl FnMut(&TimerEvent)) {
        let mut retired = RetiredEvents::default();
        retired.push_node(self.into_node());
        retired.finish_with(before_dispose);
    }

    pub(crate) fn enqueue(self) {
        let _setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
        if let Some(state) = &self.node.event.cancellation {
            state.owner_lp.store(crate::cpu::isa::lp::ops::get_lp_id(), Ordering::Release);
        }
        // No allocation or admission remains after the caller publishes Blocked
        // or a completion record. The mask restores IRQs after the queue guard.
        let retired = {
            let mut queue =
                TIMER_QUEUES.try_get_mut().expect("local timer queue is already borrowed");
            queue.add_prepared(self.into_node())
        };
        retired.finish();
    }
}

pub(crate) fn thread_timer_context(
    tid: crate::cpu::scheduler::threads::ThreadId,
) -> Option<(crate::cpu::scheduler::threads::ThreadGeneration, budget::SchedulerSponsor)> {
    let table = crate::cpu::scheduler::threads::MASTER_THREAD_TABLE.read();
    table.get(tid).ok().map(|thread| (thread.generation, thread.timer_sponsor.clone()))
}

pub(crate) fn prepare_watchdog(
    duration: ExtDuration,
    sponsor: &budget::SchedulerSponsor,
) -> Result<(PreparedEvent, TimerEventCancelHandle), ()> {
    let (event, handle) = TimerEvent::charged(duration, sponsor.reserve()?)?;
    match PreparedEvent::new(event) {
        Ok(event) => Ok((event, handle)),
        Err(error) => {
            handle.release();
            Err(error)
        }
    }
}

/// Cancel a previously enqueued cancellable event.
///
/// The returned owner retains the exact handle and any detached payloads until
/// explicit finish after both local and enclosing queue/registry guards leave.
/// Its Boolean describes only removal of the owning queue's exact entry.
/// False also covers unpublished/selected events, a busy queue, remote callers
/// and already-fired events. The shared flag suppresses notification whose
/// signal check has not started, including an event in a captured due prefix.
/// A false result is not completion, quiescence or permission to replay work.
pub(crate) fn cancel_event(handle: TimerEventCancelHandle) -> CancelledTimer {
    let mut cancelled = CancelledTimer::new(handle);
    let handle = cancelled.handle();
    handle.state.cancelled.store(true, Ordering::Release);
    if crate::cpu::isa::lp::ops::get_lp_id() != handle.state.owner_lp.load(Ordering::Acquire) {
        return cancelled;
    }

    let interrupts_were_enabled = crate::cpu::isa::lp::ops::get_int_state();
    mask_interrupts!();
    // Selection transfers an event into its delivery owner before notification.
    // Its retained state still suppresses delivery that has not started; failed
    // queue removal is not an operation completion or quiescence result.
    let (removed, retired) = match TIMER_QUEUES.try_get_mut() {
        Ok(mut queue) => queue.remove_cancellable_event(&handle.state),
        Err(_) => (false, RetiredEvents::default()),
    };
    cancelled.record(removed, retired);
    if interrupts_were_enabled {
        unmask_interrupts!();
    }
    cancelled
}

/// Reconcile the comparator and deliver one captured due prefix on this LP.
/// Both IRQ adapters share this path. Nested drains defer to the active owner;
/// callbacks never retain the queue borrow.
pub fn process_local_events() {
    try_process_local_events().expect("local timer queue is already borrowed");
}

/// Busy is queue-borrow rejection, not operation completion. An active
/// delivery keeps its original owner and comparator until explicit handback.
pub(crate) fn try_process_local_events()
-> Result<(), crate::cpu::multiprocessor::spin::per_lp::Error> {
    let _mask = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
    let pending = TIMER_QUEUES.try_get_mut()?.prepare_delivery();
    if let Some(pending) = pending {
        pending.deliver();
    }
    Ok(())
}

/// Identity for timer events that must have at most one queued instance.
///
/// Ordinary sleeps and completion timers are anonymous. The scheduler quantum
/// is different: every dispatch resets the same per-LP deadline, so its
/// identity belongs in the timer queue rather than in a separate `armed` bit
/// that can drift out of sync with the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerEventKey {
    SchedulerQuantum,
}

/// A timer event that should notify observers when a specified deadline is reached. The deadline
/// can be set using either a duration or an absolute timestamp.
#[derive(Debug)]
pub struct TimerEvent {
    deadline: Timestamp,
    key: Option<TimerEventKey>,
    cancellation: Option<CancellationRef>,
    // Internal timer producers each install exactly one callback. Embed that
    // slot instead of allocating an unbounded weak-observer queue.
    callback: Mutex<Option<Weak<dyn Observer>>>,
    waiters: WaiterSource,
    // Shared with the node's outside owner and every cancellation reference.
    _charge: Option<EventAdmission>,
}

#[derive(Debug)]
struct TimerEventCancellation {
    owner_lp: AtomicU32,
    cancelled: AtomicBool,
}

impl TimerEvent {
    pub(crate) fn prepare_sleep(
        self,
        sponsor: &budget::SchedulerSponsor,
    ) -> Result<PreparedEvent, ()> {
        event_preparation::sleep(self, sponsor)
    }

    #[inline(always)]
    pub fn get_deadline(&self) -> Timestamp {
        self.deadline
    }

    pub(crate) fn keyed(duration: ExtDuration, key: TimerEventKey) -> Self {
        let mut event = Self::from(duration);
        event.key = Some(key);
        event
    }

    pub(crate) fn charged(
        duration: ExtDuration,
        charge: budget::Charge,
    ) -> Result<(Self, TimerEventCancelHandle), ()> {
        Self::charged_with(charge, || deadline_after(duration))
    }

    /// Retain an operation's original deadline across unlocked preparation.
    pub(crate) fn charged_at(
        deadline: Timestamp,
        charge: budget::Charge,
    ) -> Result<(Self, TimerEventCancelHandle), ()> {
        Self::charged_with(charge, || deadline)
    }

    fn charged_with(
        charge: budget::Charge,
        deadline: impl FnOnce() -> Timestamp,
    ) -> Result<(Self, TimerEventCancelHandle), ()> {
        event_preparation::cancellable(charge, deadline)
    }

    /// Rebase a relative event after its observer and blocked state have been
    /// installed. This is used by scheduler sleep, whose contract is to block
    /// for at least the requested duration; computing the deadline before
    /// contended scheduler locks can make a short sleep expire before its
    /// caller has actually parked.
    pub(crate) fn reset_after(&mut self, duration: ExtDuration) {
        self.deadline = deadline_after(duration);
    }

    fn signal(&self) {
        if self.cancellation.as_ref().is_some_and(|state| state.cancelled.load(Ordering::Acquire)) {
            return;
        }
        // Detach both forms before callbacks; never enter the scheduler under
        // either source guard. Owning entries release their charge before wake.
        let callback = self.callback.lock().take();
        let waiters = self.waiters.drain();
        if let Some(observer) = callback.and_then(|observer| observer.upgrade()) {
            observer.notify();
        }
        waiters.notify();
    }
}

impl From<Timestamp> for TimerEvent {
    fn from(deadline: Timestamp) -> Self {
        Self {
            deadline,
            key: None,
            cancellation: None,
            callback: Mutex::new(None),
            waiters: WaiterSource::new(),
            _charge: None,
        }
    }
}

impl From<ExtDuration> for TimerEvent {
    fn from(duration: ExtDuration) -> Self {
        Self {
            deadline: deadline_after(duration),
            key: None,
            cancellation: None,
            callback: Mutex::new(None),
            waiters: WaiterSource::new(),
            _charge: None,
        }
    }
}

pub(crate) fn deadline_after(duration: ExtDuration) -> Timestamp {
    charlotte_lifecycle::saturating_timer_deadline(
        LpTimer::now(),
        duration.as_picos() / LpTimer::get_ts_cycle_period().as_picos(),
    )
}

impl TimerEvent {
    /// One trusted internal callback, separate from scheduler waiter admission.
    #[inline]
    pub(crate) fn register_observer(&self, observer: Weak<dyn Observer>) {
        let mut slot = self.callback.lock();
        assert!(slot.is_none(), "timer event already has its internal callback");
        *slot = Some(observer);
    }
}

impl Observable for TimerEvent {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        self.waiters.register(observer, sponsor)
    }
}

#[derive(Debug, Default)]
pub struct TimerQueue {
    events: queue::Events,
    // Exactly one PendingDelivery owns this inline claim. Abandonment retains
    // the fence; neither Drop nor a fresh object grants recovery authority.
    delivering: bool,
}

impl TimerQueue {
    fn record_cancelled(&self, count: usize) {
        if count != 0 {
            TIMER_DIAGNOSTICS[crate::cpu::isa::lp::ops::get_lp_id() as usize]
                .anonymous_cancelled
                .fetch_add(count as u64, Ordering::Relaxed);
        }
    }

    fn purge_cancelled(&mut self, retired: &mut RetiredEvents) -> usize {
        let before = self.events.len();
        self.events.retain(retired, |event| {
            !event
                .cancellation
                .as_ref()
                .is_some_and(|state| state.cancelled.load(Ordering::Acquire))
        });
        let removed = before - self.events.len();
        self.record_cancelled(removed);
        removed
    }

    fn record_state(&self) {
        let lp = crate::cpu::isa::lp::ops::get_lp_id() as usize;
        if let Some(diag) = TIMER_DIAGNOSTICS.get(lp) {
            diag.queue_len.store(self.events.len() as u64, Ordering::Relaxed);
            diag.anonymous_queued.store(
                self.events.iter().filter(|event| event.key.is_none()).count() as u64,
                Ordering::Relaxed,
            );
            diag.front_deadline
                .store(self.events.front().map_or(0, |event| event.deadline), Ordering::Relaxed);
            diag.sampled_now.store(LpTimer::now(), Ordering::Relaxed);
        }
    }

    /// Queue a keyed event only when no event with that identity is already
    /// present. The queue is the source of truth, while an existing quantum's
    /// deadline is deliberately preserved across voluntary yields.
    pub(crate) fn ensure_event(&mut self, event: TimerEvent) -> RetiredEvents {
        let mut retired = RetiredEvents::default();
        self.purge_cancelled(&mut retired);
        let key = event.key.expect("ensure_event requires a keyed event");
        if self.events.iter().any(|queued| queued.key == Some(key)) {
            // Software presence does not prove that the LP comparator is
            // still programmed. In particular, the initial quantum can be
            // queued before local interrupt-controller initialization resets
            // or masks the hardware timer. Reconcile without moving the
            // existing deadline; a past deadline becomes the minimal prompt
            // timeout in `rearm_front`.
            self.rearm_front();
            self.record_state();
            retired.push_quantum(event);
            return retired;
        }
        self.add_event(event);
        retired
    }

    pub(crate) fn remove_event(&mut self, key: TimerEventKey) -> RetiredEvents {
        let mut retired = RetiredEvents::default();
        self.purge_cancelled(&mut retired);
        self.events.retain(&mut retired, |queued| queued.key != Some(key));
        self.rearm_front();
        self.record_state();
        retired
    }

    fn remove_cancellable_event(
        &mut self,
        cancellation: &CancellationRef,
    ) -> (bool, RetiredEvents) {
        let mut retired = RetiredEvents::default();
        let before = self.events.len();
        // The consuming handle retains this allocation throughout lookup and
        // removal. Its address cannot be reused while borrowed here; equal LP,
        // deadline or cancellation flags never substitute for object identity.
        self.events.retain(&mut retired, |event| {
            event.cancellation.as_ref().is_none_or(|state| !Arc::ptr_eq(state, cancellation))
        });
        let removed = before != self.events.len();
        self.record_cancelled(
            if removed {
                1
            } else {
                0
            },
        );
        if removed {
            self.rearm_front();
            self.record_state();
        }
        (removed, retired)
    }

    fn add_prepared(&mut self, node: queue::OwnedNode) -> RetiredEvents {
        let mut retired = RetiredEvents::default();
        self.purge_cancelled(&mut retired);
        if node
            .event
            .cancellation
            .as_ref()
            .is_some_and(|state| state.cancelled.load(Ordering::Acquire))
        {
            self.rearm_front();
            self.record_state();
            retired.push_node(node);
            return retired;
        }
        self.events.insert_prepared(node);
        self.after_insert(true, None);
        retired
    }

    fn add_event(&mut self, event: TimerEvent) {
        // Only ensure_event publishes the uncharged inline quantum; anonymous
        // preparation/cancellation retains its complete admitted node instead.
        assert!(event.cancellation.is_none() && event._charge.is_none());
        let key = event.key.expect("anonymous timer insertion requires a prepared node");
        self.events.insert_quantum(event);
        self.after_insert(false, Some(key));
    }

    fn after_insert(&self, is_anonymous: bool, key: Option<TimerEventKey>) {
        let diag = &TIMER_DIAGNOSTICS[crate::cpu::isa::lp::ops::get_lp_id() as usize];
        if is_anonymous {
            diag.anonymous_added.fetch_add(1, Ordering::Relaxed);
        } else {
            diag.keyed_added.fetch_add(1, Ordering::Relaxed);
        }
        debug_assert!(
            self.events
                .iter()
                .zip(self.events.iter().skip(1))
                .all(|(left, right)| left.deadline <= right.deadline),
            "timer queue lost deadline ordering"
        );
        if let Some(key) = key {
            debug_assert_eq!(
                self.events.iter().filter(|queued| queued.key == Some(key)).count(),
                1,
                "keyed timer event is not unique"
            );
        }
        // Always reconcile after insertion. This keeps software and hardware
        // state together even after an earlier interrupt/queue interleaving.
        self.rearm_front();
        self.record_state();
    }

    pub(crate) fn prepare_delivery(&mut self) -> Option<PendingDelivery> {
        assert!(
            !crate::cpu::isa::lp::ops::get_int_state(),
            "timer selection requires caller IRQ mask"
        );
        if self.delivering {
            return None;
        }
        self.delivering = true;
        let mut retired = RetiredEvents::default();
        self.purge_cancelled(&mut retired);
        let mut ready = RetiredEvents::default();
        // One monotonic cutoff bounds delivery to this already-admitted prefix.
        // Reentrant enqueue cannot extend it or restart an operation deadline.
        let cutoff = LpTimer::now();
        while let Some(event) = self.events.front().filter(|event| event.deadline <= cutoff) {
            let is_anonymous = event.key.is_none();
            crate::debug_trace::trace(
                crate::debug_trace::TAG_TIMER_FIRED,
                event.deadline,
                self.events.len() as u64,
                0,
            );
            self.events.pop_front_into(&mut ready);
            let diag = &TIMER_DIAGNOSTICS[crate::cpu::isa::lp::ops::get_lp_id() as usize];
            if is_anonymous {
                diag.anonymous_fired.fetch_add(1, Ordering::Relaxed);
            } else {
                diag.keyed_fired.fetch_add(1, Ordering::Relaxed);
            }
        }
        ready.reverse_nodes();
        self.rearm_front();
        self.record_state();
        Some(PendingDelivery::new(ready, retired))
    }

    fn complete_delivery(&mut self) {
        assert!(
            !crate::cpu::isa::lp::ops::get_int_state(),
            "timer handback requires caller IRQ mask"
        );
        assert!(self.delivering, "timer delivery claim lost");
        self.delivering = false;
        // Callbacks may have changed the queue or advanced beyond its future
        // deadline. Reconcile current state, never restore a captured head.
        self.rearm_front();
        self.record_state();
    }

    fn get_next_deadline(&self) -> Option<Timestamp> {
        self.events.front().map(|event| event.deadline)
    }

    fn rearm_front(&self) {
        let timer = LpTimer::get();
        let mut timerlk = timer.lock();
        let _ = timerlk.stop();
        let Some(next_event) = self.events.front() else {
            crate::debug_trace::trace(crate::debug_trace::TAG_TIMER_STOPPED, 0, 0, 0);
            return;
        };
        match timerlk.set_deadline(next_event.deadline) {
            Ok(()) => {}
            Err(LpTimerError::DeadlinePassed) => {
                let _ = timerlk.set_duration(ExtDuration::from_nanos(1));
            }
            Err(e) => panic!("Failed to set timer deadline for new event: {e:?}"),
        }
        timerlk.start().expect("Failed to start timer for new event");
        crate::debug_trace::trace(
            crate::debug_trace::TAG_TIMER_ARMED,
            next_event.deadline,
            self.events.len() as u64,
            0,
        );
    }
}
