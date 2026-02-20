use alloc::collections::btree_set::BTreeSet;

use crate::cpu::scheduler::threads::{MASTER_THREAD_TABLE, ThreadId};

#[derive(Debug, PartialEq, Eq)]
struct ThreadHandle(ThreadId);
impl PartialOrd for ThreadHandle {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        let self_as = unsafe {
            MASTER_THREAD_TABLE.try_get_element_arc(self.0).unwrap_unchecked().read().asid
        };
        let other_as = unsafe {
            MASTER_THREAD_TABLE.try_get_element_arc(other.0).unwrap_unchecked().read().asid
        };
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

pub struct RoundRobinLocalSched {
    run_queue: BTreeSet<ThreadHandle>,
}
