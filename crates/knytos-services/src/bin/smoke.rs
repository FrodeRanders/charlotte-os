//! Minimal ring-3 smoke service: writes a sentinel to the status page, emits
//! one kernel log line through the syscall ABI, and exits. This is the
//! smallest possible Rust ELF that exercises the full `catten-rt` startup
//! path (config-page launch header, status page, syscall dispatch) without
//! any name-service or IPC dependency.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Context,
    config,
};
use catten_syscall::{
    el0_log,
    thread_exit,
};
use charlotte_launch::smoke_status as status;

fn main(_ctx: Context) -> ! {
    config::write::<u32>(status::MARKER, 0xdead_beef);
    el0_log(0xbeef, 0x1234);
    unsafe { thread_exit() };
}

catten_rt::entry!(main);
