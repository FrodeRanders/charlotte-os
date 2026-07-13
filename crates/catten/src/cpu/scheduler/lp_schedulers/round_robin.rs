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
    memory::{
        AddressSpaceId,
        KERNEL_ASID,
    },
    timers::{
        TIMER_QUEUES,
        TimerEvent,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThreadHandle(ThreadId);
impl PartialOrd for ThreadHandle {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        let tt_guard = MASTER_THREAD_TABLE.read();
        let self_as = unsafe { tt_guard.get(self.0).as_ref().unwrap_unchecked().asid };
        let other_as = unsafe { tt_guard.get(other.0).as_ref().unwrap_unchecked().asid };
        // Sort first by AddressSpaceId then by ThreadId
        if self_as != other_as {
            self_as.partial_cmp(&other_as)
        } else {
            self.0.partial_cmp(&other.0)
        }
    }
}
impl Ord for ThreadHandle {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        unsafe { self.partial_cmp(other).unwrap_unchecked() }
    }
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
            is_idle: false,
            timer_event_observer: Arc::new(super::TimerEventObserver::new()),
            run_queue: VecDeque::new(),
            current_handle: None,
            hwasid_map: HashMap::new(),
            idle_tid,
        }
    }

    fn set_next_timer_event(&self) {
        // Keep exactly one quantum event in flight. If one is already armed,
        // do not enqueue another — otherwise manual `yield_lp` calls (which
        // also clear the pending flag) would each add a quantum event and the
        // timer queue would grow without bound.
        if self
            .timer_event_observer
            .armed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let mut timer_event = TimerEvent::from(self.quantum);
        timer_event.register_observer(
            Arc::downgrade(&self.timer_event_observer) as alloc::sync::Weak<dyn Observer>
        );
        TIMER_QUEUES.try_get_mut().unwrap().add_event(timer_event);
    }
}

impl LpScheduler for RoundRobin {
    fn get_lp_id(&self) -> crate::cpu::isa::lp::LpId {
        self.lp_id
    }

    fn get_tid(&self) -> Option<ThreadId> {
        if let Some(th) = self.current_handle {
            Some(th.0)
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
        self.set_next_timer_event();
    }

    fn next(&mut self) -> Result<ThreadId, Error> {
        let previous_handle = self.current_handle;
        // A thread that blocked itself remains `current_handle` until this
        // point so that `cond_yield_lp` can save its execution context. It must
        // NOT be re-queued or marked Ready — it is Blocked and will be
        // re-admitted only when its waker fires. The idle thread is likewise
        // never re-queued: it is the fallback, not a normal run-queue member.
        let previous_blocked = if let Some(handle) = previous_handle {
            matches!(
                MASTER_THREAD_TABLE.read().get(handle.0),
                Ok(t) if matches!(t.state, ThreadState::Blocked(_))
            )
        } else {
            false
        };
        let previous_is_idle = matches!(previous_handle, Some(h) if h.0 == self.idle_tid);
        let requeue_previous = previous_handle.is_some() && !previous_blocked && !previous_is_idle;

        if requeue_previous {
            self.run_queue.push_back(unsafe { previous_handle.unwrap_unchecked() });
        }

        // Prefer a real runnable thread; otherwise fall back to this LP's idle
        // thread. Falling back to idle (rather than returning the outgoing
        // thread) is what lets a self-blocked or exited sole thread be switched
        // away from correctly instead of being spuriously resumed.
        let next_handle = match self.run_queue.pop_front() {
            Some(handle) => handle,
            None => ThreadHandle(self.idle_tid),
        };
        self.current_handle = Some(next_handle);
        let next_tid = next_handle.0;

        let mut tt_guard = MASTER_THREAD_TABLE.write();
        if requeue_previous {
            tt_guard
                .get_mut(unsafe { previous_handle.unwrap_unchecked() }.0)
                .as_mut()
                .unwrap()
                .state = ThreadState::Ready(self.lp_id);
        }
        tt_guard.get_mut(next_tid).as_mut().unwrap().state = ThreadState::Running(self.lp_id);
        Ok(next_tid)
    }

    fn add_thread(&mut self, tid: ThreadId) -> Result<(), Error> {
        let mut tt_guard = MASTER_THREAD_TABLE.write();
        let thread = match tt_guard.get_mut(tid) {
            Ok(t) => t,
            // The thread was removed (e.g. exited via THREAD_EXIT) before a
            // late-arriving observer notification could re-admit it. Harmless.
            Err(_) => return Err(Error::InvalidThread),
        };
        match thread.state {
            ThreadState::Running(_) | ThreadState::Ready(_) => {
                Err(Error::ThreadAlreadyAssignedToLp)
            }
            ThreadState::NeedsLpAssignment | ThreadState::Blocked(_) => {
                thread.state = ThreadState::Ready(self.lp_id);
                self.run_queue.push_back(ThreadHandle(tid));
                Ok(())
            }
        }
    }

    fn remove_thread(&mut self, tid: ThreadId) -> Result<(), Error> {
        let handle = ThreadHandle(tid);

        if self.current_handle == Some(handle) {
            self.current_handle = None;
            Ok(())
        } else {
            match self.run_queue.iter().position(|queued| *queued == handle) {
                Some(idx) => {
                    self.run_queue.remove(idx);
                    Ok(())
                }
                None => Err(Error::ThreadNotAssignedToThisLp),
            }
        }
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
    }

    fn asid_to_hwasid(&self, asid: AddressSpaceId) -> Option<HwAsid> {
        self.hwasid_map.get(&asid).cloned()
    }

    fn thread_count(&self) -> ThreadCount {
        // The idle thread is not real work and must not skew load balancing.
        let current_is_real = matches!(self.current_handle, Some(h) if h.0 != self.idle_tid);
        self.run_queue.len()
            + if current_is_real {
                1
            } else {
                0
            }
    }
}
