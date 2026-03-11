pub mod round_robin;
use alloc::fmt::Debug;

use crate::cpu::isa::lp::LpId;
use crate::cpu::isa::memory::paging::HwAsid;
use crate::cpu::scheduler::threads::{ThreadCount, ThreadId};
use crate::memory::AddressSpaceId;

pub trait LpScheduler: Debug + Send {
    fn get_lp_id(&self) -> LpId;
    fn get_tid(&self) -> Option<ThreadId>;
    fn next(&mut self) -> Result<ThreadId, Error>;
    fn add_thread(&mut self, tid: ThreadId) -> Result<(), Error>;
    fn remove_thread(&mut self, tid: ThreadId) -> Result<(), Error>;
    fn is_idle(&self) -> bool;
    fn asid_to_hwasid(&self, asid: AddressSpaceId) -> Option<HwAsid>;
    fn thread_count(&self) -> ThreadCount;
}

#[derive(Debug)]
pub enum Error {
    EmptyRunQueue,
    ThreadAlreadyAssignedToLp,
    ThreadNotAssignedToThisLp,
}
