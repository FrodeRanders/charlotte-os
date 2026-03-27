use alloc::boxed::Box;
use alloc::collections::vec_deque::VecDeque;
use alloc::format;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU16, Ordering};

use spin::mutex::Mutex;

use crate::cpu::isa::interface::memory::address::VirtualAddress;
use crate::cpu::isa::lp::LpId;
use crate::cpu::isa::lp::ops::{get_lp_local_base, set_lp_local_base};
use crate::cpu::scheduler::threads::ThreadId;
use crate::memory::{AddressSpaceId, VAddr};

const AS_AFFINITY_COUNT: usize = 4095;

pub struct LogicalProcessor {
    pub id: LpId,
    pub as_affinities: [Option<AddressSpaceId>; AS_AFFINITY_COUNT],
    pub exec_queue_ptr: Arc<Mutex<VecDeque<ThreadId>>>,
    pub interrupt_depth: AtomicU16,
}

impl LogicalProcessor {
    pub fn setup(id: LpId, exec_queue_ptr: Arc<Mutex<VecDeque<ThreadId>>>) {
        let lp_struct = Box::try_new(LogicalProcessor {
            id,
            as_affinities: [None; AS_AFFINITY_COUNT],
            exec_queue_ptr,
            interrupt_depth: AtomicU16::new(0),
        })
        .expect(&format!(
            "Failed to allocate an LP struct for LP {id}. Main memory is insufficient for core \
             kernel functionality."
        ));
        let vaddr = VAddr::from_mut(Box::into_raw(lp_struct));
        set_lp_local_base(vaddr);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn increment_interrupt_depth() {
    let lp_struct = get_lp_local_base().into_mut::<LogicalProcessor>();
    unsafe {
        (*lp_struct).interrupt_depth.fetch_add(1, Ordering::AcqRel);
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn decrement_interrupt_depth() {
    let lp_struct = get_lp_local_base().into_mut::<LogicalProcessor>();
    unsafe {
        (*lp_struct).interrupt_depth.fetch_sub(1, Ordering::AcqRel);
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn get_interrupt_depth() -> u16 {
    let lp_struct = get_lp_local_base().into_mut::<LogicalProcessor>();
    unsafe { (*lp_struct).interrupt_depth.load(Ordering::Acquire) }
}
