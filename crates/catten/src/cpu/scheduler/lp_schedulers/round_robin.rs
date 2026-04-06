use alloc::collections::vec_deque::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::AtomicBool;

use hashbrown::HashMap;

use crate::cpu::isa::lp::LpId;
use crate::cpu::isa::lp::ops::{get_lp_id, mask_interrupts, unmask_interrupts};
use crate::cpu::isa::memory::paging::HwAsid;
use crate::cpu::scheduler::lp_schedulers::{Error, LpScheduler};
use crate::cpu::scheduler::threads::{MASTER_THREAD_TABLE, ThreadCount, ThreadId, ThreadState};
use crate::klib::observer::{Observable, Observer};
use crate::klib::time::duration::ExtDuration;
use crate::memory::AddressSpaceId;
use crate::timers::{TIMER_QUEUES, TimerEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThreadHandle(ThreadId);
impl PartialOrd for ThreadHandle {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        mask_interrupts!();
        let self_as =
            unsafe { MASTER_THREAD_TABLE.read().get(self.0).as_ref().unwrap_unchecked().asid };
        let other_as =
            unsafe { MASTER_THREAD_TABLE.read().get(other.0).as_ref().unwrap_unchecked().asid };
        unmask_interrupts!();
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
    ctx_switch_pending: AtomicBool,
    timer_event_observer: Arc<super::TimerEventObserver>,
    run_queue: VecDeque<ThreadHandle>,
    current_handle: Option<ThreadHandle>,
    hwasid_map: HashMap<AddressSpaceId, HwAsid>,
}

impl RoundRobin {
    pub fn new(lp_id: LpId, quantum: ExtDuration) -> Self {
        Self {
            lp_id,
            quantum,
            is_idle: false,
            ctx_switch_pending: AtomicBool::new(false),
            timer_event_observer: Arc::new(super::TimerEventObserver {}),
            run_queue: VecDeque::new(),
            current_handle: None,
            hwasid_map: HashMap::new(),
        }
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
        self.ctx_switch_pending.load(core::sync::atomic::Ordering::Acquire)
    }

    fn set_ctx_switch_pending(&self) {
        unsafe {
            let raw_self = core::mem::transmute::<&RoundRobin, *mut RoundRobin>(self);
            (*raw_self).ctx_switch_pending.store(true, core::sync::atomic::Ordering::Release);
        }
    }

    fn clear_ctx_switch_pending(&self) {
        unsafe {
            let raw_self = core::mem::transmute::<&RoundRobin, *mut RoundRobin>(self);
            (*raw_self).ctx_switch_pending.store(false, core::sync::atomic::Ordering::Release);
        }
        let mut timer_event = TimerEvent::from(self.quantum);
        timer_event.register_observer(
            Arc::downgrade(&self.timer_event_observer) as alloc::sync::Weak<dyn Observer>
        );
        TIMER_QUEUES.try_get_mut().unwrap().add_event(timer_event);
    }

    fn next(&mut self) -> Result<ThreadId, Error> {
        if self.run_queue.is_empty() {
            Err(Error::EmptyRunQueue)
        } else {
            if let Some(handle) = self.current_handle {
                self.run_queue.push_back(handle);
            }
            self.current_handle = Some(unsafe { self.run_queue.pop_front().unwrap_unchecked() });
            let next_tid = unsafe { self.current_handle.unwrap_unchecked() }.0;
            // Update the thread's state value in the master thread table.
            // Note: callers (yield_lp) must ensure interrupts are masked before calling next()
            // to prevent re-entrant deadlocks from pending IPIs.
            MASTER_THREAD_TABLE.write().get_mut(next_tid).as_mut().unwrap().state =
                ThreadState::Running(get_lp_id());
            Ok(next_tid)
        }
    }

    fn add_thread(&mut self, tid: ThreadId) -> Result<(), Error> {
        match MASTER_THREAD_TABLE.read().get(tid).as_ref().unwrap().state {
            ThreadState::Running(_) | ThreadState::Ready(_) => {
                Err(Error::ThreadAlreadyAssignedToLp)
            }
            _ => {
                let new_handle = ThreadHandle(tid);
                self.run_queue
                    .insert(self.run_queue.partition_point(|e| *e < new_handle), new_handle);
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
            match self.run_queue.binary_search(&handle) {
                Ok(idx) => {
                    self.run_queue.remove(idx);
                    Ok(())
                }
                Err(_) => Err(Error::ThreadNotAssignedToThisLp),
            }
        }
    }

    fn is_idle(&self) -> bool {
        self.is_idle
    }

    fn asid_to_hwasid(&self, asid: AddressSpaceId) -> Option<HwAsid> {
        self.hwasid_map.get(&asid).cloned()
    }

    fn thread_count(&self) -> ThreadCount {
        self.run_queue.len()
            + if self.current_handle.is_some() {
                1
            } else {
                0
            }
    }
}
