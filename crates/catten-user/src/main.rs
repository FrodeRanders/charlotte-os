//! CharlotteOS sitas runtime demonstration.
//!
//! Uses the `sitas-charlotte` `CharlotteReactor` to exercise the async syscall
//! ABI end-to-end: submit an operation via the kernel's COMPLETION_SUBMIT
//! syscall, wait for the CQ ring to show a completion, read the result, and
//! write a sentinel to the result page.

#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

use core::panic::PanicInfo;
use sitas_charlotte::CharlotteReactor;

const RESULT_PAGE: *mut u32 = 0x0000_0000_0001_2000usize as *mut u32;

/// Write a u32 to the result page (mapped at 0x12000 in the user AS).
unsafe fn write_result(value: u32) {
    unsafe { core::ptr::write_volatile(RESULT_PAGE, value) };
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // Create a reactor for the test user AS (asid=1) on LP 0.
    let reactor = CharlotteReactor::new(1, 0);

    // Submit a NOP operation via COMPLETION_SUBMIT (syscall #1).
    // The kernel test pre-populates the CQ ring with one entry.
    let cap = reactor.submit_wait(0, None);
    unsafe { write_result(cap as u32) };

    // Spin-poll the CQ ring until a completion arrives.
    let mut seen: u32 = 0;
    for _ in 0..10_000_000 {
        let pending = reactor.cq().pending();
        if pending > 0 {
            if let Some(entry) = reactor.cq().read_one() {
                seen = entry.result as u64 as u32;
                break;
            }
        }
    }
    unsafe { write_result(seen) };

    loop {
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)) };
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)) };
    }
}
