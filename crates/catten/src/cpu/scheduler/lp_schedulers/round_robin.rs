use alloc::{
    collections::vec_deque::VecDeque,
    sync::Arc,
};
use core::sync::atomic::Ordering;

use hashbrown::HashMap;

use crate::{
    cpu::{
        isa::{
            lp::LpId,
            memory::paging::HwAsid,
        },
        scheduler::{
            lp_schedulers::{
                Error,
                LpScheduler,
            },
            threads::{
                MASTER_THREAD_TABLE,
                Thread,
                ThreadCount,
                ThreadGeneration,
                ThreadId,
                ThreadState,
            },
        },
    },
    klib::{
        observer::{
            Observable,
            Observer,
        },
        time::duration::ExtDuration,
    },
    logln,
    memory::{
        AddressSpaceId,
        KERNEL_ASID,
    },
    timers::{
        TIMER_QUEUES,
        TimerEvent,
        TimerEventKey,
    },
};

const SCHED_TRACE: bool = false;

macro_rules! sched_trace {
    ($($arg:tt)*) => {
        if SCHED_TRACE {
            logln!($($arg)*);
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThreadHandle {
    tid: ThreadId,
    generation: ThreadGeneration,
}

#[derive(Debug)]
pub struct RoundRobin {
    lp_id: LpId,
    quantum: ExtDuration,
    is_idle: bool,
    timer_event_observer: Arc<super::TimerEventObserver>,
    run_queue: VecDeque<ThreadHandle>,
    current_handle: Option<ThreadHandle>,
    hwasid_map: HashMap<AddressSpaceId, HwAsid>,
    /// This LP's dedicated idle thread, run only when there is no other
    /// runnable thread. It exists so that a thread which blocks itself (or
    /// exits) while it is the sole runnable thread on this LP is switched away
    /// from — saving its context correctly — rather than being spuriously
    /// resumed. It is per-LP because a single thread context cannot be run on
    /// two LPs at once (the `on_cpu` hand-off in `switch_ctx` would serialise
    /// them). It is never placed in `run_queue`; `next` returns it as a
    /// fallback only.
    idle_tid: ThreadId,
}

impl RoundRobin {
    pub fn new(lp_id: LpId, quantum: ExtDuration) -> Self {
        // Create this LP's dedicated idle thread up front and register it in the
        // master thread table. It is never submitted to a run queue; it is only
        // ever returned by `next` as the fallback when nothing else is runnable.
        let idle_tid = {
            let idle = Thread::new(KERNEL_ASID, crate::cpu::isa::lp::ops::lp_idle_loop);
            MASTER_THREAD_TABLE.write().add_element(idle)
        };
        Self {
            lp_id,
            quantum,
            // No thread has been dispatched and no quantum is armed yet.
            // Initial remote submission must therefore send a wakeup IPI.
            is_idle: true,
            timer_event_observer: Arc::new(super::TimerEventObserver::new()),
            run_queue: VecDeque::new(),
            current_handle: None,
            hwasid_map: HashMap::new(),
            idle_tid,
        }
    }

    fn set_next_timer_event(&self) {
        let timer_event = TimerEvent::keyed(self.quantum, TimerEventKey::SchedulerQuantum);
        timer_event.register_observer(
            Arc::downgrade(&self.timer_event_observer) as alloc::sync::Weak<dyn Observer>
        );
        TIMER_QUEUES.try_get_mut().unwrap().ensure_event(timer_event);
    }
}

impl LpScheduler for RoundRobin {
    fn get_lp_id(&self) -> crate::cpu::isa::lp::LpId {
        self.lp_id
    }

    fn get_tid(&self) -> Option<ThreadId> {
        if let Some(th) = self.current_handle {
            Some(th.tid)
        } else {
            None
        }
    }

    fn is_ctx_switch_pending(&self) -> bool {
        self.timer_event_observer.pending.load(Ordering::Acquire)
    }

    fn set_ctx_switch_pending(&self) {
        self.timer_event_observer.pending.store(true, Ordering::Release);
    }

    fn clear_ctx_switch_pending(&self) {
        self.timer_event_observer.pending.store(false, Ordering::Release);
        // An idle LP is woken explicitly when work is admitted, so periodic
        // round-robin ticks only waste host CPU and immediately return to the
        // same idle thread. Arm a quantum only for real runnable work.
        if !self.is_idle {
            self.set_next_timer_event();
        } else {
            TIMER_QUEUES.try_get_mut().unwrap().remove_event(TimerEventKey::SchedulerQuantum);
        }
    }

    fn next(&mut self) -> Result<ThreadId, Error> {
        let previous_handle = self.current_handle;
        // A thread that blocked itself remains `current_handle` until this
        // point so that `cond_yield_lp` can save its execution context. It must
        // NOT be re-queued or marked Ready — it is Blocked and will be
        // re-admitted only when its waker fires. The idle thread is likewise
        // never re-queued: it is the fallback, not a normal run-queue member.
        //
        // Re-queue the outgoing thread only while it is still `Running`: that
        // is the ordinary preemption case. If its state is already `Ready`, a
        // waker re-admitted it concurrently (for example a sleep timer or CQ
        // wake firing in the window between `block_thread` and this context
        // switch) and it is already in a run queue — re-queueing it here would
        // double-enqueue the thread and corrupt its state machine.
        let previous_running = if let Some(handle) = previous_handle {
            matches!(
                MASTER_THREAD_TABLE.read().get(handle.tid),
                Ok(t) if t.generation == handle.generation
                    && matches!(t.state, ThreadState::Running(_))
            )
        } else {
            false
        };
        let previous_is_idle = matches!(previous_handle, Some(h) if h.tid == self.idle_tid);
        let requeue_previous = previous_running && !previous_is_idle;

        if requeue_previous {
            self.run_queue.push_back(unsafe { previous_handle.unwrap_unchecked() });
        }

        // Prefer a real runnable thread; otherwise fall back to this LP's idle
        // thread. Falling back to idle (rather than returning the outgoing
        // thread) is what lets a self-blocked or exited sole thread be switched
        // away from correctly instead of being spuriously resumed.
        let next_handle = loop {
            match self.run_queue.pop_front() {
                Some(handle) => {
                    let valid = MASTER_THREAD_TABLE
                        .read()
                        .get(handle.tid)
                        .is_ok_and(|thread| thread.generation == handle.generation);
                    if valid {
                        break handle;
                    }
                    // A stale handle must never schedule a later occupant of
                    // the recycled numeric thread id.
                }
                None => {
                    let generation =
                        MASTER_THREAD_TABLE.read().get(self.idle_tid).unwrap().generation;
                    break ThreadHandle {
                        tid: self.idle_tid,
                        generation,
                    };
                }
            }
        };
        self.current_handle = Some(next_handle);
        let next_tid = next_handle.tid;
        let became_idle = next_tid == self.idle_tid;
        self.is_idle = became_idle;

        let mut tt_guard = MASTER_THREAD_TABLE.write();
        if requeue_previous {
            tt_guard
                .get_mut(unsafe { previous_handle.unwrap_unchecked() }.tid)
                .as_mut()
                .unwrap()
                .state = ThreadState::Ready(self.lp_id);
        }
        tt_guard.get_mut(next_tid).as_mut().unwrap().state = ThreadState::Running(self.lp_id);

        sched_trace!(
            "[sched] LP{} dispatch: out={:?} requeue={} in={} depth={} idle={}",
            self.lp_id,
            previous_handle,
            requeue_previous,
            next_tid,
            self.run_queue.len(),
            became_idle
        );
        crate::debug_trace::trace(
            crate::debug_trace::TAG_SCHED_DISPATCH,
            previous_handle.map_or(u64::MAX, |handle| handle.tid as u64),
            next_tid as u64,
            ((self.run_queue.len() as u64) << 1) | became_idle as u64,
        );

        Ok(next_tid)
    }

    fn add_thread(
        &mut self,
        tid: ThreadId,
        expected_generation: Option<ThreadGeneration>,
    ) -> Result<(), Error> {
        let mut tt_guard = MASTER_THREAD_TABLE.write();
        let thread = match tt_guard.get_mut(tid) {
            Ok(t) => t,
            // The thread was removed (e.g. exited via THREAD_EXIT) before a
            // late-arriving observer notification could re-admit it. Harmless.
            Err(_) => return Err(Error::InvalidThread),
        };
        if expected_generation.is_some_and(|generation| generation != thread.generation) {
            return Err(Error::InvalidThread);
        }
        let handle = ThreadHandle {
            tid,
            generation: thread.generation,
        };
        match thread.state {
            // Already runnable. A wake that aggregates several sources onto one
            // thread (the unified shard wait of architecture doc §7: CQ
            // completions, endpoint readiness, device interrupts, and explicit
            // peer wakes all target the same blocked thread) can fire more than
            // once before the thread next parks. Re-admitting an
            // already-runnable thread is therefore a benign no-op, not an error
            // — it must not double-enqueue it and must not panic.
            ThreadState::Running(_) => {
                sched_trace!(
                    "[sched] LP{} add TID={} gen={} already-Running (noop)",
                    self.lp_id,
                    tid,
                    thread.generation
                );
                Ok(())
            }
            ThreadState::Ready(_) => {
                sched_trace!(
                    "[sched] LP{} add TID={} gen={} already-Ready (noop)",
                    self.lp_id,
                    tid,
                    thread.generation
                );
                Ok(())
            }
            ThreadState::NeedsLpAssignment | ThreadState::Blocked(_) => {
                thread.state = ThreadState::Ready(self.lp_id);
                self.run_queue.push_back(handle);
                crate::debug_trace::trace(
                    crate::debug_trace::TAG_SCHED_ADMIT,
                    tid as u64,
                    thread.generation,
                    self.run_queue.len() as u64,
                );
                sched_trace!(
                    "[sched] LP{} add TID={} gen={} -> Ready depth={}",
                    self.lp_id,
                    tid,
                    thread.generation,
                    self.run_queue.len()
                );
                Ok(())
            }
        }
    }

    fn remove_thread(&mut self, tid: ThreadId) -> Result<(), Error> {
        let handle = self.run_queue.iter().find(|handle| handle.tid == tid).copied();

        if self.current_handle.is_some_and(|handle| handle.tid == tid) {
            self.current_handle = None;
            Ok(())
        } else {
            match handle
                .and_then(|handle| self.run_queue.iter().position(|queued| *queued == handle))
            {
                Some(idx) => {
                    self.run_queue.remove(idx);
                    Ok(())
                }
                None => Err(Error::ThreadNotAssignedToThisLp),
            }
        }
    }

    fn ready_migration_candidates(&self) -> alloc::vec::Vec<(ThreadId, ThreadGeneration)> {
        self.run_queue.iter().map(|handle| (handle.tid, handle.generation)).collect()
    }

    fn remove_ready_for_migration(
        &mut self,
        tid: ThreadId,
        generation: ThreadGeneration,
    ) -> Result<(), Error> {
        let position = self
            .run_queue
            .iter()
            .position(|handle| handle.tid == tid && handle.generation == generation)
            .ok_or(Error::ThreadNotAssignedToThisLp)?;
        self.run_queue.remove(position);
        Ok(())
    }

    fn add_ready_from_migration(
        &mut self,
        tid: ThreadId,
        generation: ThreadGeneration,
    ) -> Result<(), Error> {
        if self.run_queue.iter().any(|handle| handle.tid == tid && handle.generation == generation)
        {
            return Err(Error::ThreadAlreadyAssignedToLp);
        }
        self.run_queue.push_back(ThreadHandle {
            tid,
            generation,
        });
        Ok(())
    }

    fn is_idle(&self) -> bool {
        self.is_idle
    }

    fn start(&mut self) {
        self.is_idle = false;
        self.set_next_timer_event();
    }

    fn stop(&mut self) {
        self.is_idle = true;
        TIMER_QUEUES.try_get_mut().unwrap().remove_event(TimerEventKey::SchedulerQuantum);
    }

    fn asid_to_hwasid(&self, asid: AddressSpaceId) -> Option<HwAsid> {
        self.hwasid_map.get(&asid).cloned()
    }

    fn thread_count(&self) -> ThreadCount {
        // The idle thread is not real work and must not skew load balancing.
        let current_is_real = matches!(self.current_handle, Some(h) if h.tid != self.idle_tid);
        self.run_queue.len()
            + if current_is_real {
                1
            } else {
                0
            }
    }
}
