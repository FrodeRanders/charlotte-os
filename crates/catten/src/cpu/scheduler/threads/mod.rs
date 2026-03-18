use alloc::boxed::Box;
use alloc::vec::Vec;
use core::mem::offset_of;
use core::sync::atomic::AtomicPtr;

use spin::rwlock::RwLock;
use spin::{Lazy, Mutex};

use crate::common::collections::id_table::IdTable;
use crate::cpu::isa::lp::LpId;
use crate::cpu::isa::lp::thread_context::ThreadContext;
use crate::event::Completion;
use crate::memory::{AddressSpaceId, VAddr};

pub static MASTER_THREAD_TABLE: Lazy<RwLock<ThreadTable>> =
    Lazy::new(|| RwLock::new(ThreadTable::new()));
pub type ThreadTable = IdTable<Mutex<Box<Thread>>>;
pub type ThreadId = usize;

pub type ThreadCount = usize;

#[derive(Debug)]
pub enum ThreadState {
    Running(LpId),
    Ready(LpId),
    NeedsLpAssignment,
    Blocked(Vec<Completion>),
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
    pub fn new(is_user: bool, asid: AddressSpaceId, entry_point: VAddr) -> Self {
        Thread {
            is_user,
            context: ThreadContext::new(asid, entry_point).expect("Error creating thread context"),
            asid,
            state: ThreadState::NeedsLpAssignment,
        }
    }

    pub unsafe fn get_ctx_ptr(&self) -> *mut ThreadContext {
        (&raw const self.context) as *mut ThreadContext
    }
}
