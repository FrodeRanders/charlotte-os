//! CharlotteOS sitas spawn test — exercises SVC #7 (SPAWN_THREAD).
//!
//! Calls the SPAWN_THREAD syscall to create a kernel thread on LP 0 that runs
//! a small test function. The spawned thread writes a sentinel to the result
//! page; the main thread polls for the sentinel and signals success.

#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

use core::panic::PanicInfo;

const RESULT_PAGE: *mut u32 = 0x0000_0000_0001_2000usize as *mut u32;

/// The function the spawned thread will execute. Writes the sentinel 0xCAFE
/// to the result page.
unsafe fn thread_entry() {
    unsafe { core::ptr::write_volatile(RESULT_PAGE, 0xCAFEu32) };
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // The raw binary is loaded at 0x10000. Compute thread_entry's offset from
    // the start of the binary and construct the correct virtual address.
    let base = 0x10000usize;
    let entry_offset = unsafe { thread_entry as usize } - unsafe { _start as usize };
    let entry_vaddr = base + entry_offset;

    // Call SVC #7 (SPAWN_THREAD).
    // Arguments: x0=asid(1), x1=entry_vaddr, x2=target_lp(0)
    let tid: u64;
    unsafe {
        core::arch::asm!(
            "svc #7",
            inlateout("x0") 1u64 => tid,
            in("x1") entry_vaddr as u64,
            in("x2") 0u64,
            options(nostack, nomem, preserves_flags),
        );
    }

    // Spin-poll the result page for the sentinel from the spawned thread.
    for _ in 0..10_000_000 {
        let sentinel = unsafe { core::ptr::read_volatile(RESULT_PAGE) };
        if sentinel == 0xCAFE {
            unsafe { core::ptr::write_volatile(RESULT_PAGE, tid as u32) };
            break;
        }
        core::hint::spin_loop();
    }

    loop {
        unsafe { core::hint::spin_loop() };
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe { core::hint::spin_loop() };
    }
}
