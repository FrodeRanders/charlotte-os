//! The greeting service: the cluster's deployed artifact.
//!
//! This is the binary the cluster-deployment demo actually ships. The
//! pipeline signs it (`tools/cluster-sign elf-sign`) with the cluster's
//! private key and stages the note-signed ELF in the service bundle as
//! `greet.elf`; that signed ELF is the artifact stored in the object store,
//! fetched by the deploy agent, verified against the cluster public key, and
//! served across the network. The signature note is also what lets the EL0
//! loader accept it as a loadable domain.
//!
//! When run, it registers itself under the `deploy` interface and answers
//! `OP_GET` with the greeting value.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    deploy,
    ns,
    wait_reply,
};
use catten_syscall::{
    IpcRights,
    ipc_endpoint_create,
    ipc_recv_block,
    ipc_reply,
    ipc_scalar_call_connection,
    ipc_status,
    thread_exit,
};

const REPLY_SPINS: u64 = 50_000_000;

fn main(ctx: Context) -> ! {
    config::write::<u32>(0, 1); // stage: started
    let ns_connection = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(0, 2); // stage: bootstrap connection received

    let endpoint = ipc_endpoint_create(deploy::INTERFACE, deploy::VERSION, 8);
    if endpoint == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 3); // stage: endpoint created

    let register = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_REGISTER,
        deploy::NAME,
        endpoint,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if register == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 4); // stage: short register call sent
    let (generation, _) = unsafe { wait_reply(register, REPLY_SPINS) };
    if generation < 1 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(4, generation as u32);
    config::write::<u32>(0, 6); // stage: serving

    loop {
        // This service owns no completion queue work other than endpoint
        // readiness. Block directly on the endpoint so every subsequent
        // invocation wakes it; an unbound `cq_wait` after the first request
        // left the service asleep forever.
        let message = ipc_recv_block(endpoint);
        if message.status == ipc_status::ENDPOINT_CLOSED {
            unsafe { thread_exit() };
        }
        if !message.is_ok() {
            continue;
        }
        match message.opcode {
            deploy::OP_GET => {
                // The deployed artifact's greeting: the leading eight bytes
                // of the artifact (its ELF header), little-endian.
                if message.reply != 0 {
                    ipc_reply(message.reply, deploy::GREET_VALUE as i64);
                }
            }
            _ => {
                if message.reply != 0 {
                    ipc_reply(message.reply, 0);
                }
            }
        }
    }
}

catten_rt::entry!(main);
