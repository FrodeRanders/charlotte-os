//! The reference console client (architecture doc §10, Phase 8).
//!
//! Bootstrap → look up the "uart" console service by name → write a short
//! message through it → query the driver's interrupt count. Never learns
//! kernel identifiers; reaches the device only through a delegated console
//! connection. Results are posted to the config-page output words for the
//! kernel verifier:
//!
//! - `config[1]` (byte 4, u32): last `OP_WRITE` reply status
//! - `config[2]` (byte 8, u32): driver-reported interrupt count
//! - `config[3]` (byte 12, u32): progress stage
//! - `config[0]` (byte 0, u32): sentinel `0xC0DE`, written last
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{Args, Input, config};
use catten_services::{console, ns, wait_reply};
use catten_syscall::{ipc_scalar_call, thread_exit};

const REPLY_SPINS: u64 = 50_000_000;
const READ_SPINS: u64 = 400_000_000;
const LOOKUP_ATTEMPTS: u64 = 1_000_000;
const SENTINEL: u32 = 0xC0DE;

/// Config-page output words (console-client domain).
const SENTINEL_OFFSET: usize = 0;
const WRITE_STATUS_OFFSET: usize = 4;
const IRQ_COUNT_OFFSET: usize = 8;
const STAGE_OFFSET: usize = 12;
const READ_RESULT_OFFSET: usize = 40;

/// The message the client writes through the console driver.
const MESSAGE: &[u8] = b"UART-OK\n";

fn cmain(_args: Args, _input: Input<0>) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1);
    let ns_connection = match config::bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(STAGE_OFFSET, 2);

    let mut attempts: u64 = 0;
    let console_connection = loop {
        let lookup = ipc_scalar_call(ns_connection, ns::OP_LOOKUP, console::NAME);
        if lookup != 0 {
            let (result, cap) = unsafe { wait_reply(lookup, REPLY_SPINS) };
            if result >= 1 && cap != 0 {
                break cap;
            }
        }
        attempts += 1;
        if attempts >= LOOKUP_ATTEMPTS {
            unsafe { thread_exit() };
        }
        core::hint::spin_loop();
    };
    config::write::<u32>(STAGE_OFFSET, 3);

    let mut last_status: i64 = 0;
    for &byte in MESSAGE {
        let call = ipc_scalar_call(console_connection, console::OP_WRITE, byte as u64);
        if call == 0 {
            unsafe { thread_exit() };
        }
        let (status, _) = unsafe { wait_reply(call, REPLY_SPINS) };
        last_status = status;
    }
    config::write::<u32>(WRITE_STATUS_OFFSET, last_status as u32);
    config::write::<u32>(STAGE_OFFSET, 4);

    let status_call = ipc_scalar_call(console_connection, console::OP_STATUS, 0);
    if status_call == 0 {
        unsafe { thread_exit() };
    }
    let (irq_count, _) = unsafe { wait_reply(status_call, REPLY_SPINS) };
    config::write::<u32>(IRQ_COUNT_OFFSET, irq_count as u32);

    let read_call = ipc_scalar_call(console_connection, console::OP_READ_DEFERRED, 0);
    if read_call == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 5);
    let (read_result, _) = unsafe { wait_reply(read_call, READ_SPINS) };
    config::write::<u32>(READ_RESULT_OFFSET, read_result as u32);

    config::write::<u32>(SENTINEL_OFFSET, SENTINEL);
    unsafe { thread_exit() };
}

catten_rt::entry!(cmain);
