pub mod round_robin;
use alloc::fmt::Debug;

use crate::cpu::isa::memory::paging::HwAsid;
use crate::cpu::scheduler::threads::ThreadId;
use crate::memory::AddressSpaceId;

pub trait LpScheduler: Debug {
    type ThreadHandle: Debug + PartialOrd + Ord;
    type Error: Debug;

    fn next(&mut self) -> Result<ThreadId, Self::Error>;
    fn add_thread(&mut self, tid: ThreadId) -> Result<(), Self::Error>;
    fn remove_thread(&mut self, tid: ThreadId) -> Result<(), Self::Error>;
    fn remove_as(&mut self, asid: AddressSpaceId) -> Result<(), Self::Error>;
    fn is_idle(&self) -> bool;
    fn asid_to_hwasid(&self, asid: AddressSpaceId) -> Option<HwAsid>;
    fn thread_count(&self) -> u64;
}
