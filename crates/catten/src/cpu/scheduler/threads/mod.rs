use core::mem::offset_of;

use spin::Lazy;
use spin::rwlock::RwLock;

use crate::cpu::isa::lp::LpId;
use crate::cpu::isa::lp::thread_context::ThreadContext;
use crate::klib::collections::id_table::IdTable;
use crate::memory::AddressSpaceId;

pub static MASTER_THREAD_TABLE: Lazy<RwLock<ThreadTable>> =
    Lazy::new(|| RwLock::new(ThreadTable::new()));
pub type ThreadTable = IdTable<Thread>;
pub type ThreadId = usize;

pub type ThreadCount = usize;

#[derive(Debug)]
pub enum ThreadState {
    Running(LpId),
    Ready(LpId),
    NeedsLpAssignment,
    Blocked,
    Terminated, //Used while the thread is being cleaned up
}

#[derive(Debug)]
pub struct Thread {
    pub is_user: bool,
    pub context: ThreadContext,
    pub asid: AddressSpaceId,
    pub state: ThreadState,
}

pub const THREAD_CTX_OFFSET: usize = offset_of!(Thread, context);

impl Thread {
    pub fn new(is_user: bool, asid: AddressSpaceId, entry_point: *const fn()) -> Self {
        Thread {
            is_user,
            context: if is_user {
                ThreadContext::new_us(asid, entry_point)
                    .expect("Error creating user thread context")
            } else {
                ThreadContext::new_ks(entry_point).expect("Error creating kernel thread context")
            },
            asid,
            state: ThreadState::NeedsLpAssignment,
        }
    }
}
