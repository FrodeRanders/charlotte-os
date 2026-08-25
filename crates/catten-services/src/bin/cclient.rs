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
    wait_for_registered_name_owned,
};
use catten_syscall::thread_exit;
use charlotte_launch::uart_client_status as status;

const SENTINEL: u32 = 0xc0de;

/// The message the client writes through the console driver.
const MESSAGE: &[u8] = b"UART-OK\n";

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let ns_connection = match ctx.bootstrap_connection() {
        Some(connection) => connection,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 2);

    let (_, console_connection) = wait_for_registered_name_owned(ns_connection, console::NAME)
        .unwrap_or_else(|| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 3);

    let mut last_status: i64 = 0;
    for &byte in MESSAGE {
        last_status = console_connection
            .call(console::OP_WRITE, byte as u64)
            .and_then(|call| call.wait())
            .unwrap_or_else(|_| unsafe { thread_exit() })
            .result;
    }
    config::write::<u32>(status::WRITE_STATUS, last_status as u32);
    config::write::<u32>(status::STAGE, 4);

    let irq_count = console_connection
        .call(console::OP_STATUS, 0)
        .and_then(|call| call.wait())
        .unwrap_or_else(|_| unsafe { thread_exit() })
        .result;
    config::write::<u32>(status::IRQ_COUNT, irq_count as u32);

    let read_call = console_connection
        .call(console::OP_READ_DEFERRED, 0)
        .unwrap_or_else(|_| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 5);
    let read_result = read_call.wait().unwrap_or_else(|_| unsafe { thread_exit() }).result;
    config::write::<u32>(status::READ_RESULT, read_result as u32);

    config::write::<u32>(status::SENTINEL, SENTINEL);
    unsafe { thread_exit() };
}

catten_rt::entry!(main);
