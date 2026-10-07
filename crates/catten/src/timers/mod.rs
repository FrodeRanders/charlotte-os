//! Per-LP sorted timer queue, backed by the ARM Generic Timer.
//!
//! Each LP has a [`TimerQueue`] — a sorted owning list of anonymous events and
//! one inline scheduler quantum. Their earliest deadline programs the hardware
//! comparator; the PPI interrupt fires at
//! that tick, `process_events` drains all expired events, and the comparator
//! is re-armed with the next deadline.
//!
//! Queue mutation reconciles the comparator with the earliest deadline.
//! Cancelled anonymous timers retain their admission charge until the owning
//! queue removes them. The idle loop calls `process_events`
//! before `wfi` to reconcile the software queue with the hardware comparator
//! and prevent missed deadlines after timer transitions.

pub(crate) mod budget;
pub(crate) mod event_tests;
mod queue;
pub(crate) mod waiter_tests;

use alloc::{
    boxed::Box,
    sync::{
        Arc,
        Weak,
    },
};
use core::sync::atomic::{
    AtomicBool,
    AtomicU32,
    AtomicU64,
    Ordering,
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

static NEXT_TIMER_EVENT_ID: AtomicU64 = AtomicU64::new(1);

/// A capability to cancel one anonymous timer event.
///
/// Timer queues are LP-local because their head programs LP-local hardware.
/// The shared flag prevents notification even if a future scheduler change
/// resumes the waiter on a different LP; the owner removes the event eagerly
/// whenever cancellation happens on its original LP.
pub(crate) struct TimerEventCancelHandle {
    state: Arc<TimerEventCancellation, EventAdmission>,
}

// One event reservation covers its node and cancellation backing. Clones used
// as lifetime owners never allocate; only the cancellation Arc uses its single
// allocation allowance. The node carries an outside owner through Box release.
type EventAdmission = ChargedAllocator<budget::Charge>;

pub type Timestamp = <LpTimer as LpTimerIfce>::Timestamp;

/// Owning, fallibly prepared anonymous queue node. Preparation allocates but
/// holds no queue guard or interrupt mask; callers mask park + publication.
/// Enqueue additionally masks its local comparator transaction and updates
/// shared cancellation ownership to the actual publishing LP.
pub(crate) struct PreparedEvent {
    node: queue::OwnedNode,
}

impl PreparedEvent {
    pub(crate) fn new(event: TimerEvent) -> Result<Self, ()> {
        Self::new_with(event, |node| Box::try_new(node).map_err(|_| ()))
    }

    fn new_with(
        event: TimerEvent,
        allocate: impl FnOnce(queue::Node) -> Result<Box<queue::Node>, ()>,
    ) -> Result<Self, ()> {
        assert!(event.key.is_none() && event._charge.is_some());
        let admission = event._charge.as_ref().unwrap().clone();
        let node = queue::OwnedNode::new(allocate(queue::Node::new(event))?, admission);
        Ok(Self {
            node,
        })
    }

    pub(crate) fn event(&self) -> &TimerEvent {
        &self.node.event
    }

    pub(crate) fn event_mut(&mut self) -> &mut TimerEvent {
        &mut self.node.event
    }

    pub(crate) fn enqueue(self) {
        let _setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
        if let Some(state) = &self.node.event.cancellation {
            state.owner_lp.store(crate::cpu::isa::lp::ops::get_lp_id(), Ordering::Release);
        }
        // No allocation or admission remains after the caller publishes Blocked
        // or a completion record. The mask restores IRQs after the queue guard.
        TIMER_QUEUES
            .try_get_mut()
            .expect("local timer queue is already borrowed")
            .add_prepared(self.node);
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
    Ok((PreparedEvent::new(event)?, handle))
}

/// Cancel a previously enqueued cancellable event.
///
/// Returns whether this call removed the event from its owning queue. A false
/// result means it already fired, or that the caller migrated away from the
/// LP whose hardware timer owns it. In the latter case the shared cancellation
/// flag still suppresses notification and the owner purges it on its next
/// queue operation.
pub(crate) fn cancel_event(handle: TimerEventCancelHandle) -> bool {
    handle.state.cancelled.store(true, Ordering::Release);
    if crate::cpu::isa::lp::ops::get_lp_id() != handle.state.owner_lp.load(Ordering::Acquire) {
        return false;
    }

    let interrupts_were_enabled = crate::cpu::isa::lp::ops::get_int_state();
    mask_interrupts!();
    // A completion may release its cancellation owner from a timer callback.
    // That callback already holds this LP's queue borrow: flag cancellation
    // and let the outer queue operation reclaim the event instead of re-entering.
    let removed = TIMER_QUEUES
        .try_get_mut()
        .is_ok_and(|mut queue| queue.remove_cancellable_event(handle.state.id));
    if interrupts_were_enabled {
        unmask_interrupts!();
    }
    removed
}

/// Reconcile due events and the hardware comparator from thread context.
/// The IRQ dispatcher calls `TimerQueue::process_events` directly because its
/// exception entry has already masked local IRQs; idle-loop callers use this
/// wrapper to obtain the same non-reentrant transaction.
pub fn process_local_events() {
    let interrupts_were_enabled = crate::cpu::isa::lp::ops::get_int_state();
    mask_interrupts!();
    {
        TIMER_QUEUES.try_get_mut().expect("local timer queue is already borrowed").process_events();
    }
    if interrupts_were_enabled {
        unmask_interrupts!();
    }
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
    cancellation: Option<Arc<TimerEventCancellation, EventAdmission>>,
    // Internal timer producers each install exactly one callback. Embed that
    // slot instead of allocating an unbounded weak-observer queue.
    callback: Mutex<Option<Weak<dyn Observer>>>,
    waiters: WaiterSource,
    // Shared with the node's outside owner and every cancellation reference.
    _charge: Option<EventAdmission>,
}

#[derive(Debug)]
struct TimerEventCancellation {
    id: u64,
    owner_lp: AtomicU32,
    cancelled: AtomicBool,
}

impl TimerEvent {
    pub(crate) fn prepare_sleep(
        mut self,
        sponsor: &budget::SchedulerSponsor,
    ) -> Result<PreparedEvent, ()> {
        assert!(self._charge.is_none() && self.key.is_none());
        self._charge = Some(EventAdmission::try_new(sponsor.reserve()?).map_err(|_| ())?);
        PreparedEvent::new(self)
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
        let admission = EventAdmission::try_new(charge).map_err(|_| ())?;
        let id = NEXT_TIMER_EVENT_ID.fetch_add(1, Ordering::Relaxed);
        let state = admission
            .clone()
            .try_arc(TimerEventCancellation {
                id,
                owner_lp: AtomicU32::new(crate::cpu::isa::lp::ops::get_lp_id()),
                cancelled: AtomicBool::new(false),
            })
            .map_err(|_| ())?;
        let handle = TimerEventCancelHandle {
            state: state.clone(),
        };
        let event = Self {
            deadline: deadline_after(duration),
            key: None,
            cancellation: Some(state),
            callback: Mutex::new(None),
            waiters: WaiterSource::new(),
            _charge: Some(admission),
        };
        Ok((event, handle))
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

fn deadline_after(duration: ExtDuration) -> Timestamp {
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
}

impl TimerQueue {
    fn record_cancelled(&self, count: usize) {
        if count != 0 {
            TIMER_DIAGNOSTICS[crate::cpu::isa::lp::ops::get_lp_id() as usize]
                .anonymous_cancelled
                .fetch_add(count as u64, Ordering::Relaxed);
        }
    }

    fn purge_cancelled(&mut self) -> usize {
        let before = self.events.len();
        self.events.retain(|event| {
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
    pub(crate) fn ensure_event(&mut self, event: TimerEvent) {
        self.purge_cancelled();
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
            return;
        }
        self.add_event(event);
    }

    pub(crate) fn remove_event(&mut self, key: TimerEventKey) {
        self.purge_cancelled();
        self.events.retain(|queued| queued.key != Some(key));
        self.rearm_front();
        self.record_state();
    }

    fn remove_cancellable_event(&mut self, id: u64) -> bool {
        let before = self.events.len();
        self.events.retain(|event| event.cancellation.as_ref().is_none_or(|state| state.id != id));
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
        removed
    }

    fn add_prepared(&mut self, node: queue::OwnedNode) {
        self.purge_cancelled();
        if node
            .event
            .cancellation
            .as_ref()
            .is_some_and(|state| state.cancelled.load(Ordering::Acquire))
        {
            self.rearm_front();
            self.record_state();
            return;
        }
        self.events.insert_prepared(node);
        self.after_insert(true, None);
    }

    fn add_event(&mut self, event: TimerEvent) {
        self.purge_cancelled();
        // Teardown/cancellation may win between record publication and
        // enqueue. Do not publish an already-cancelled queue node.
        if event.cancellation.as_ref().is_some_and(|state| state.cancelled.load(Ordering::Acquire))
        {
            return;
        }
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

    pub fn process_events(&mut self) {
        self.purge_cancelled();
        while let Some(event) = self.events.front() {
            if event.get_deadline() <= LpTimer::now() {
                let is_anonymous = event.key.is_none();
                crate::debug_trace::trace(
                    crate::debug_trace::TAG_TIMER_FIRED,
                    event.get_deadline(),
                    self.events.len() as u64,
                    0,
                );
                event.signal();
                self.events.pop_front();
                let diag = &TIMER_DIAGNOSTICS[crate::cpu::isa::lp::ops::get_lp_id() as usize];
                if is_anonymous {
                    diag.anonymous_fired.fetch_add(1, Ordering::Relaxed);
                } else {
                    diag.keyed_fired.fetch_add(1, Ordering::Relaxed);
                }
                self.record_state();
            } else if let Some(deadline) = self.get_next_deadline() {
                let timer = LpTimer::get();
                let mut timerlk = timer.lock();
                if timerlk.set_deadline(deadline) == Err(LpTimerError::DeadlinePassed) {
                    continue;
                }
                timerlk.start().expect("Failed to start timer for next event");
                crate::debug_trace::trace(
                    crate::debug_trace::TAG_TIMER_ARMED,
                    deadline,
                    self.events.len() as u64,
                    0,
                );
                self.record_state();
                return;
            } else {
                let _ = LpTimer::get().lock().stop();
                crate::debug_trace::trace(crate::debug_trace::TAG_TIMER_STOPPED, 0, 0, 0);
                self.record_state();
                return;
            }
        }
        // The queue drained completely. Stop the timer so it does not keep
        // firing on a stale (already-passed) compare value — the ARM Generic
        // Timer interrupt is level-triggered and would otherwise re-assert.
        let _ = LpTimer::get().lock().stop();
        crate::debug_trace::trace(crate::debug_trace::TAG_TIMER_STOPPED, 0, 0, 0);
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
