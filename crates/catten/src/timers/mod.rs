//! # Kernel Timer System

use alloc::{
    collections::vec_deque::VecDeque,
    sync::Weak,
};

use concurrent_queue::ConcurrentQueue;
use spin::LazyLock;

use crate::{
    cpu::{
        isa::{
            interface::timers::{
                LpTimerError,
                LpTimerIfce,
            },
            timers::LpTimer,
        },
        multiprocessor::spin::per_lp::PerLp,
    },
    klib::{
        observer::{
            Observable,
            Observer,
        },
        time::duration::ExtDuration,
    },
};

pub static TIMER_QUEUES: LazyLock<PerLp<TimerQueue>> =
    LazyLock::new(|| PerLp::new(TimerQueue::default));

pub type Timestamp = <LpTimer as LpTimerIfce>::Timestamp;

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
    observers: ConcurrentQueue<Weak<dyn Observer>>,
}

impl TimerEvent {
    #[inline(always)]
    pub fn get_deadline(&self) -> Timestamp {
        self.deadline
    }

    pub(crate) fn keyed(duration: ExtDuration, key: TimerEventKey) -> Self {
        let mut event = Self::from(duration);
        event.key = Some(key);
        event
    }

    fn signal(&self) {
        for observer in self.observers.try_iter() {
            if let Some(observer) = observer.upgrade() {
                observer.notify();
            }
        }
    }
}

impl From<Timestamp> for TimerEvent {
    fn from(deadline: Timestamp) -> Self {
        Self {
            deadline,
            key: None,
            observers: ConcurrentQueue::unbounded(),
        }
    }
}

impl From<ExtDuration> for TimerEvent {
    fn from(duration: ExtDuration) -> Self {
        let deadline = LpTimer::now()
            + (duration.as_picos() / LpTimer::get_ts_cycle_period().as_picos()) as Timestamp;
        Self {
            deadline,
            key: None,
            observers: ConcurrentQueue::unbounded(),
        }
    }
}

impl Observable for TimerEvent {
    #[inline]
    fn register_observer(&self, observer: Weak<dyn Observer>) {
        self.observers.push(observer).expect("Failed to register observer");
    }
}

#[derive(Debug, Default)]
pub struct TimerQueue {
    events: VecDeque<TimerEvent>,
}

impl TimerQueue {
    /// Queue a keyed event only when no event with that identity is already
    /// present. The queue is the source of truth, while an existing quantum's
    /// deadline is deliberately preserved across voluntary yields.
    pub(crate) fn ensure_event(&mut self, event: TimerEvent) {
        let key = event.key.expect("ensure_event requires a keyed event");
        if self.events.iter().any(|queued| queued.key == Some(key)) {
            // Software presence does not prove that the LP comparator is
            // still programmed. In particular, the initial quantum can be
            // queued before local interrupt-controller initialization resets
            // or masks the hardware timer. Reconcile without moving the
            // existing deadline; a past deadline becomes the minimal prompt
            // timeout in `rearm_front`.
            self.rearm_front();
            return;
        }
        self.add_event(event);
    }

    pub(crate) fn remove_event(&mut self, key: TimerEventKey) {
        self.events.retain(|queued| queued.key != Some(key));
        self.rearm_front();
    }

    pub fn add_event(&mut self, event: TimerEvent) {
        let mut insertion_idx: Option<usize> = None;
        for (i, event_node) in self.events.iter().enumerate() {
            if event.deadline < event_node.get_deadline() {
                insertion_idx = Some(i);
                break;
            }
        }
        if insertion_idx.is_none() {
            // If we get here then the event we are adding has a deadline that is after all of the
            // other events in the queue so we can just add it to the back of the queue.
            insertion_idx = Some(self.events.len());
        }
        let i = insertion_idx.unwrap();
        self.events.insert(i, event);
        // Always reconcile after insertion. This keeps software and hardware
        // state together even after an earlier interrupt/queue interleaving.
        self.rearm_front();
    }

    pub fn process_events(&mut self) {
        while let Some(event) = self.events.front() {
            if event.get_deadline() <= LpTimer::now() {
                event.signal();
                self.events.pop_front();
            } else if let Some(deadline) = self.get_next_deadline() {
                let timer = LpTimer::get();
                let mut timerlk = timer.lock();
                if timerlk.set_deadline(deadline) == Err(LpTimerError::DeadlinePassed) {
                    continue;
                }
                timerlk.start().expect("Failed to start timer for next event");
                return;
            } else {
                let _ = LpTimer::get().lock().stop();
                return;
            }
        }
        // The queue drained completely. Stop the timer so it does not keep
        // firing on a stale (already-passed) compare value — the ARM Generic
        // Timer interrupt is level-triggered and would otherwise re-assert.
        let _ = LpTimer::get().lock().stop();
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
    }
}
