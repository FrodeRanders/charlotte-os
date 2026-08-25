//! The reference client: bootstrap → name lookup → service call.
//!
//! Never learns kernel identifiers: it starts with one connection to the
//! name service, obtains an echo-service connection by looking up a
//! memory-carried (long) name, and calls it. Results are posted to the named
//! fields in [`charlotte_launch::client_status`] for the kernel verifier:
//! echoed value, service generation, progress stage, and completion sentinel.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    echo,
    ns,
    stage_name_owned,
};
use catten_syscall::thread_exit;
use charlotte_launch::client_status as status;

const ECHO_VALUE: u64 = 0x1234_5678;
const SENTINEL: u32 = 0xc0de;

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1); // stage: started
    let ns_connection = match ctx.bootstrap_connection() {
        Some(connection) => connection,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 2); // stage: bootstrap connection received

    // Look up the echo service by its memory-carried (long) name. The name
    // is staged once; copy transfer preserves the client's ownership, so the
    // same memory object remains owned by this client. The name service
    // retains the reply token until the echo service registers.
    let name = match stage_name_owned(echo::LONG_NAME) {
        Some(memory) => memory,
        None => unsafe { thread_exit() },
    };
    let lookup = ns_connection
        .call_copy(ns::OP_LOOKUP_NAMED, echo::LONG_NAME.len() as u64, &name)
        .unwrap_or_else(|_| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 3); // stage: lookup pending
    let lookup = lookup.wait().unwrap_or_else(|_| unsafe { thread_exit() });
    let generation = lookup.result;
    if generation < 1 {
        unsafe { thread_exit() };
    }
    let echo_connection = lookup.connection.unwrap_or_else(|| unsafe { thread_exit() });

    config::write::<u32>(status::STAGE, 4); // stage: connection obtained

    let call = echo_connection
        .call(echo::OP_ECHO, ECHO_VALUE)
        .unwrap_or_else(|_| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 5); // stage: echo call sent
    let echoed = call.wait().unwrap_or_else(|_| unsafe { thread_exit() }).result;
    if echoed as u64 != ECHO_VALUE {
        unsafe { thread_exit() };
    }

    config::write::<u32>(status::ECHOED, echoed as u32);
    config::write::<u32>(status::GENERATION, generation as u32);
    config::write::<u32>(status::SENTINEL, SENTINEL);
    unsafe { thread_exit() };
}

catten_rt::entry!(main);
