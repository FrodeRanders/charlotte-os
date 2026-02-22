use alloc::collections::vec_deque::VecDeque;

use hashbrown::HashMap;

use crate::cpu::isa::lp::ops::{get_lp_id, mask_interrupts, unmask_interrupts};
use crate::cpu::isa::memory::paging::HwAsid;
use crate::cpu::scheduler::lp_schedulers::{Error, LpSchedulerIfce};
use crate::cpu::scheduler::threads::{MASTER_THREAD_TABLE, ThreadCount, ThreadId, ThreadState};
use crate::memory::AddressSpaceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThreadHandle(ThreadId);
impl PartialOrd for ThreadHandle {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        mask_interrupts!();
        let self_as = unsafe {
            MASTER_THREAD_TABLE.read().try_get_element_arc(self.0).unwrap_unchecked().read().asid
        };
        let other_as = unsafe {
            MASTER_THREAD_TABLE.read().try_get_element_arc(other.0).unwrap_unchecked().read().asid
        };
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

#[derive(Debug, Default)]
pub struct RoundRobin {
    run_queue: VecDeque<ThreadHandle>,
    current_handle: Option<ThreadHandle>,
    hwasid_map: HashMap<AddressSpaceId, HwAsid>,
}

impl LpSchedulerIfce for RoundRobin {
    fn next(&mut self) -> Result<ThreadId, Error> {
        if self.run_queue.is_empty() {
            Err(Error::EmptyRunQueue)
        } else {
            if let Some(handle) = self.current_handle {
                self.run_queue.push_back(handle);
            }
            self.current_handle = Some(unsafe { self.run_queue.pop_front().unwrap_unchecked() });
            let next_tid = unsafe { self.current_handle.unwrap_unchecked() }.0;
            // Update the thread's state value in the master thread table
            unsafe {
                mask_interrupts!(); // acquiring a spinlock is not interrupt safe so we mask interrupts at the CPU level until the lock is released
                MASTER_THREAD_TABLE.read().try_get_element_arc(next_tid).unwrap().write().state =
                    ThreadState::Running(get_lp_id());
                unmask_interrupts!();
            }
            Ok(next_tid)
        }
    }

    fn add_thread(&mut self, tid: ThreadId) -> Result<(), Error> {
        let thread_arc = MASTER_THREAD_TABLE.read().try_get_element_arc(tid).unwrap();
        match thread_arc.read().state {
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
        self.current_handle == None && self.run_queue.is_empty()
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
