//! CharlotteOS adder — adds kernel-placed numbers, subtracts boilerplate.
#![no_std]
#![no_main]

extern crate alloc;
use catten_syscall::*;

const RESULT_PAGE: *mut u32 = 0x0000_0000_0001_2000usize as *mut u32;
const READ_BUF:   *mut u32 = 0x0000_0000_0001_6000usize as *mut u32;

fn cmain(_asid: u64) -> ! {
    let a = unsafe { core::ptr::read_volatile(RESULT_PAGE) };
    let b = unsafe { core::ptr::read_volatile(RESULT_PAGE.add(1)) };

    let cap = unsafe { submit_read(READ_BUF as usize, 32) };
    unsafe { wait(cap); }
    let kernel_val = unsafe { core::ptr::read_volatile(READ_BUF) };

    let sum = a.wrapping_add(b).wrapping_add(kernel_val);
    unsafe { core::ptr::write_volatile(RESULT_PAGE.add(2), sum); }
    unsafe { core::ptr::write_volatile(RESULT_PAGE, 0xC0DE); }

    unsafe { close(cap); }
    unsafe { thread_exit(); }
}

catten_rt::entry!(cmain);
