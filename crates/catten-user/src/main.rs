//! CharlotteOS adder — adds kernel-placed numbers, subtracts boilerplate.
#![no_std]
#![no_main]

extern crate alloc;
use catten_rt::config;
use catten_syscall::*;

const READ_BUF: *mut u32 = 0x0000_0000_0001_6000usize as *mut u32;

fn cmain() -> ! {
    let a: u32 = unsafe { config::read(0) };
    let b: u32 = unsafe { config::read(4) };

    let cap = unsafe { submit_read(READ_BUF as usize, 32) };
    unsafe { wait(cap); }
    let kernel_val = unsafe { core::ptr::read_volatile(READ_BUF) };

    let sum = a.wrapping_add(b).wrapping_add(kernel_val);
    config::write(0, 0xC0DEu32);
    config::write(4, b);
    config::write(8, sum);

    unsafe { close(cap); }
    unsafe { thread_exit(); }
}

catten_rt::entry!(cmain);
