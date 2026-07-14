//! The reference client: bootstrap → name lookup → service call.
//!
//! Never learns kernel identifiers: it starts with one connection to the
//! name service, obtains an echo-service connection by name lookup, and
//! calls it. Results are posted to the config-page output words for the
//! kernel verifier:
//!
//! - `config[1]` (byte 4, u32): echoed value (low 32 bits)
//! - `config[2]` (byte 8, u32): observed service generation
//! - `config[0]` (byte 0, u32): sentinel `0xC0DE`, written last
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Args,
    Input,
    config,
};
use catten_services::{
    echo,
    ns,
    wait_reply,
};
use catten_syscall::{
    ipc_scalar_call,
    thread_exit,
};

const REPLY_SPINS: u64 = 50_000_000;
const LOOKUP_ATTEMPTS: u64 = 1_000_000;
const ECHO_VALUE: u64 = 0x1234_5678;
const SENTINEL: u32 = 0xC0DE;

fn cmain(_args: Args, _input: Input<0>) -> ! {
    config::write::<u32>(12, 1); // stage: started
    let ns_connection = match config::bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(12, 2); // stage: bootstrap connection received

    // The echo service may not have registered yet; retry the lookup.
    let mut attempts: u64 = 0;
    let (generation, echo_connection) = loop {
        let lookup = unsafe { ipc_scalar_call(ns_connection, ns::OP_LOOKUP, echo::NAME) };
        if lookup != 0 {
            let (result, cap) = unsafe { wait_reply(lookup, REPLY_SPINS) };
            if result >= 1 && cap != 0 {
                break (result, cap);
            }
        }
        attempts += 1;
        config::write::<u32>(12, 3); // stage: lookup pending
        if attempts >= LOOKUP_ATTEMPTS {
            unsafe { thread_exit() };
        }
        core::hint::spin_loop();
    };
    config::write::<u32>(12, 4); // stage: connection obtained

    let call = unsafe { ipc_scalar_call(echo_connection, echo::OP_ECHO, ECHO_VALUE) };
    if call == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(12, 5); // stage: echo call sent
    let (echoed, _) = unsafe { wait_reply(call, REPLY_SPINS) };
    if echoed as u64 != ECHO_VALUE {
        unsafe { thread_exit() };
    }

    config::write::<u32>(4, echoed as u32);
    config::write::<u32>(8, generation as u32);
    config::write::<u32>(0, SENTINEL);
    unsafe { thread_exit() };
}

catten_rt::entry!(cmain);
