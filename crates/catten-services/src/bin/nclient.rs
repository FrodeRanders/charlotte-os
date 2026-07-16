//! The reference net client (Phase 9).
//!
//! Bootstrap → look up the "net0" service by name → query status → post
//! MAC and result to config page for the kernel verifier.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Args,
    Input,
    config,
};
use catten_services::{
    net,
    ns,
    wait_reply,
};
use catten_syscall::{
    ipc_scalar_call,
    thread_exit,
};

const REPLY_SPINS: u64 = 50_000_000;
const LOOKUP_ATTEMPTS: u64 = 1_000_000;
const SENTINEL: u32 = 0xC0DE;
const STAGE_OFFSET: usize = 12;
const MAC_RESULT_OFFSET: usize = 4;

fn cmain(_args: Args, _input: Input<0>) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1);
    let ns_connection = match config::bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(STAGE_OFFSET, 2);

    let mut attempts: u64 = 0;
    let net_connection = loop {
        let lookup = unsafe { ipc_scalar_call(ns_connection, ns::OP_LOOKUP, net::NAME) };
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

    let status_call = unsafe { ipc_scalar_call(net_connection, net::OP_STATUS, 0) };
    if status_call == 0 {
        unsafe { thread_exit() };
    }
    let (status, _) = unsafe { wait_reply(status_call, REPLY_SPINS) };
    config::write::<u32>(MAC_RESULT_OFFSET, status as u32);

    config::write::<u32>(0, SENTINEL);
    unsafe { thread_exit() };
}

catten_rt::entry!(cmain);
