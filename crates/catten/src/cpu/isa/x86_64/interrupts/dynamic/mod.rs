use core::arch::global_asm;

use spin::LazyLock;

use crate::cpu::{
    interrupt_routing::InterruptHandler,
    isa::lp::InterruptVectorNum,
    multiprocessor::spin::per_lp::PerLp,
};

pub const DYN_VECS_PER_LP: u64 = 220;
pub const DYN_VEC_START_OFFSET: u64 = 35;
pub static DYN_IH_TABLE: LazyLock<PerLp<[Option<InterruptHandler>; DYN_VECS_PER_LP as usize]>> =
    LazyLock::new(|| PerLp::new(|| [None; DYN_VECS_PER_LP as usize]));

global_asm!(include_str!("dyn_isrs.asm"));

#[unsafe(no_mangle)]
pub extern "C" fn get_dyn_ih(vector: InterruptVectorNum) -> *const InterruptHandler {
    if let Ok(table) = DYN_IH_TABLE.try_get() {
        if let Some(ih) = table[vector as usize] {
            return ih as *const InterruptHandler;
        }
    }
    core::ptr::null()
}
