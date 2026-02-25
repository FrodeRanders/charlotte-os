use alloc::vec::Vec;

use spin::Lazy;
use spin::rwlock::RwLock;

use crate::common::collections::id_table::IdTable;
use crate::cpu::isa::lp::LpId;
use crate::cpu::isa::lp::thread_context::ThreadContext;
use crate::event::Completion;
use crate::memory::{AddressSpaceId, VAddr};

pub static MASTER_THREAD_TABLE: Lazy<RwLock<ThreadTable>> =
    Lazy::new(|| RwLock::new(ThreadTable::new()));
pub type ThreadTable = IdTable<ThreadId, Thread>;
pub type ThreadId = usize;

pub type ThreadCount = usize;

pub enum ThreadState {
    Running(LpId),
    Ready(LpId),
    NeedsLpAssignment,
    Blocked(Vec<Completion>),
    Terminated, //Used while the thread is being cleaned up
}

pub struct Thread {
    pub is_user: bool,
    pub context: ThreadContext,
    pub asid: AddressSpaceId,
    pub state: ThreadState,
}

impl Thread {
    pub fn new(is_user: bool, asid: AddressSpaceId, entry_point: VAddr) -> Self {
        Thread {
            is_user,
            context: ThreadContext::new(asid, entry_point).unwrap(),
            asid,
            state: ThreadState::NeedsLpAssignment,
        }
    }
}
