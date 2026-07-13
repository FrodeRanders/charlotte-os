//! Minimal CharlotteOS userspace program — compiled Rust, no sitas dependency.
//! Exercises COMPLETION_SUBMIT, writes returned cap + sentinel to result page.
//!
//! Build:
//!   cargo +nightly build --manifest-path crates/catten-user/Cargo.toml \
//!       --target crates/catten-user/aarch64-unknown-none.json \
//!       -Z build-std=core
//!   SYS=$(rustc +nightly --print sysroot)
//!   $SYS/lib/rustlib/aarch64-apple-darwin/bin/llvm-objcopy -O binary \
//!       target/aarch64-unknown-none/debug/catten-user catten-user.bin
#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

use core::panic::PanicInfo;

const RESULT_PAGE: *mut u32 = 0x0000_0000_0001_2000usize as *mut u32;

#[inline(always)]
unsafe fn svc_submit(asid: u64, op_code: u64) -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "svc #1",
            inlateout("x0") asid => ret,
            in("x1") op_code,
            options(nostack, nomem, preserves_flags),
        );
    }
    ret
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // Read asid from result[4] (written by kernel during test setup).
    let asid: u32;
    unsafe {
        asid = core::ptr::read_volatile(RESULT_PAGE.add(4));
    }

    // COMPLETION_SUBMIT(asid, OpCode::Nop)
    let cap = unsafe { svc_submit(asid as u64, 0) };

    // Write cap + sentinel to result page (verifier asserts these).
    unsafe {
        core::ptr::write_volatile(RESULT_PAGE.add(1), cap as u32);
        core::ptr::write_volatile(RESULT_PAGE, 0xDEAD);
    }

    // Spin forever (or use THREAD_EXIT once verified).
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // Write panic sentinel so the verifier can distinguish a crash.
    unsafe { core::ptr::write_volatile(RESULT_PAGE, 0xBAD0_0000); }
    loop { core::hint::spin_loop(); }
}
