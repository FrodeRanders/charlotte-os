//! The reference console client (architecture doc §10, Phase 8).
//!
//! Bootstrap → look up the "uart" console service by name → write a short
//! message through it → query the driver's interrupt count. Never learns
//! kernel identifiers; reaches the device only through a delegated console
//! connection. Results are posted to the named fields in
//! [`charlotte_launch::uart_client_status`] for the kernel verifier: write
//! result, interrupt count, progress stage, deferred-read result, and
//! completion sentinel.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    console,
    ns,
    wait_reply,
};
use catten_syscall::{
    ipc_scalar_call,
    thread_exit,
};
use charlotte_launch::uart_client_status as status;

const REPLY_SPINS: u64 = 50_000_000;
const READ_SPINS: u64 = 400_000_000;
const SENTINEL: u32 = 0xc0de;

/// The message the client writes through the console driver.
const MESSAGE: &[u8] = b"UART-OK\n";

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let ns_connection = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 2);

    let lookup = ipc_scalar_call(ns_connection, ns::OP_LOOKUP, console::NAME);
    if lookup == 0 {
        unsafe { thread_exit() };
    }
    let (result, console_connection) = catten_services::spin_reply(lookup);
    if result < 1 || console_connection == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 3);

    let mut last_status: i64 = 0;
    for &byte in MESSAGE {
        let call = ipc_scalar_call(console_connection, console::OP_WRITE, byte as u64);
        if call == 0 {
            unsafe { thread_exit() };
        }
        let (status, _) = unsafe { wait_reply(call, REPLY_SPINS) };
        last_status = status;
    }
    config::write::<u32>(status::WRITE_STATUS, last_status as u32);
    config::write::<u32>(status::STAGE, 4);

    let status_call = ipc_scalar_call(console_connection, console::OP_STATUS, 0);
    if status_call == 0 {
        unsafe { thread_exit() };
    }
    let (irq_count, _) = unsafe { wait_reply(status_call, REPLY_SPINS) };
    config::write::<u32>(status::IRQ_COUNT, irq_count as u32);

    let read_call = ipc_scalar_call(console_connection, console::OP_READ_DEFERRED, 0);
    if read_call == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 5);
    let (read_result, _) = unsafe { wait_reply(read_call, READ_SPINS) };
    config::write::<u32>(status::READ_RESULT, read_result as u32);

    config::write::<u32>(status::SENTINEL, SENTINEL);
    unsafe { thread_exit() };
}

catten_rt::entry!(main);
