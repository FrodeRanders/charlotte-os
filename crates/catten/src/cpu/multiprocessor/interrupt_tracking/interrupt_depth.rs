use core::sync::atomic::{AtomicU16, Ordering};

use ::spin::Lazy;

use crate::cpu::multiprocessor::spin::per_lp::PerLp;

/// The current interrupt depth for each LP. This is used primarily to avoid self deadlocks.
static INT_DEPTH: Lazy<PerLp<AtomicU16>> = Lazy::new(|| PerLp::new(|| AtomicU16::new(0)));

#[unsafe(no_mangle)]
pub extern "C" fn increment_interrupt_depth() {
    let int_depth = &INT_DEPTH
        .try_get_mut()
        .expect("Failed to get mutable reference to PerLp<AtomicU16> for interrupt depth");
    int_depth.fetch_add(1, Ordering::AcqRel);
}
#[unsafe(no_mangle)]
pub extern "C" fn decrement_interrupt_depth() {
    let int_depth = &INT_DEPTH
        .try_get_mut()
        .expect("Failed to get mutable reference to PerLp<AtomicU16> for interrupt depth");
    int_depth.fetch_sub(1, Ordering::AcqRel);
}
#[unsafe(no_mangle)]
pub extern "C" fn get_interrupt_depth() -> u16 {
    let int_depth = &INT_DEPTH
        .try_get_mut()
        .expect("Failed to get mutable reference to PerLp<AtomicU16> for interrupt depth");
    int_depth.load(Ordering::Acquire)
}
