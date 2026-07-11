//! CharlotteOS test user binary — exercises the async syscall ABI from EL0.
#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

use core::panic::PanicInfo;

macro_rules! svc {
    ($imm:literal, x0=$x0:expr, x1=$x1:expr) => {{
        let x0: u64 = $x0;
        let x1: u64 = $x1;
        let ret: u64;
        unsafe {
            core::arch::asm!(
                concat!("svc #", stringify!($imm)),
                inlateout("x0") x0 => ret,
                in("x1") x1,
                options(nostack, nomem, preserves_flags),
            );
        }
        ret
    }};
    ($imm:literal) => {{
        unsafe {
            core::arch::asm!(
                concat!("svc #", stringify!($imm)),
                options(nostack, nomem, preserves_flags),
            );
        }
    }};
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let asid: u64 = 1;

    // Submit a NOP operation (syscall #1: COMPLETION_SUBMIT).
    // x0=asid, x1=op_code (0=Nop).
    let _cap: u64 = svc!(1, x0 = asid, x1 = 0);

    // Poll to drain the result (syscall #3: COMPLETION_POLL).
    // x0=asid, x1=cap. The cap returned from submit was in x0.
    let _poll: u64 = svc!(3, x0 = asid, x1 = 0);

    // Write a sentinel to the result page (mapped at 0x12000 in the user AS).
    let ptr = 0x0001_2000usize as *mut u32;
    unsafe { core::ptr::write_volatile(ptr, 0xDEAD) };

    // Loop forever.
    loop {
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
    }
}
