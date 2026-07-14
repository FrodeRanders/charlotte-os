//! CharlotteOS sitas integration demo — exercises the full async runtime.
//!
//! Creates a `CharlotteReactor` (implements sitas's `ReactorBackend`),
//! uses it to submit an operation via the kernel's COMPLETION_SUBMIT syscall,
//! waits for the kernel to complete it, and polls the CQ ring for the result.
//! The result is written to the result page at 0x12000 as a u64 sentinel.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

use sitas_charlotte::CharlotteReactor;

const RESULT_PAGE: *mut u64 = 0x0000_0000_0001_2000usize as *mut u64;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // Create a reactor on LP 0. The kernel derives ASID from the running
    // thread, so the legacy constructor's ASID slot is intentionally unused.
    let reactor = CharlotteReactor::new(0, 0);

    // Submit a NOP operation. The kernel completes it because the test
    // pre-populates the AS with a CQ ring and a capability table.
    //
    // Syscall #1 (COMPLETION_SUBMIT): x0 is unused, x1=op_code (0=Nop), x2=buf, x3=len.
    unsafe {
        #[allow(unsafe_op_in_unsafe_fn)]
        core::arch::asm!(
            "svc #1",
            in("x0") 0u64,
            in("x1") 0u64,
            in("x2") 0u64,
            in("x3") 0u64,
            options(nostack, nomem),
        );
    }

    // Wait for the CQ ring to show a completion. The kernel's `complete()`
    // call in the EL0 test pre-fills one entry, so this should spin until
    // the entry is visible.
    let mut seen = 0u64;
    for _ in 0..10_000_000 {
        let pending = reactor.cq().pending();
        if pending > 0 {
            if let Some(entry) = reactor.cq().read_one() {
                seen = entry.result as u64;
                break;
            }
        }
    }

    // Write the result to the result page so the kernel test can read it.
    unsafe { core::ptr::write_volatile(RESULT_PAGE, seen) };

    // Loop forever.
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
