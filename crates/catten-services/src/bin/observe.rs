//! CharlotteOS owned-snapshot observability service.
//!
//! This first version exposes scheduler statistics for its own address space
//! through endpoint IPC. It deliberately cannot inspect other domains:
//! services retain ownership of their telemetry until they explicitly publish
//! it or receive a future delegated system-observer capability.
#![no_std]
#![no_main]

catten_rt::entry!(main);

use catten_rt::Context;
use catten_services::{
    ns,
    observability,
};
use catten_syscall::{
    IpcRights,
    cq_wait,
    ipc_close,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_reply_wait,
    ipc_scalar_call_connection,
    ipc_status,
    thread_exit,
    thread_statistics_snapshot,
};

fn main(ctx: Context) -> ! {
    let ns_connection = ctx.bootstrap_cap().unwrap_or_else(|| unsafe { thread_exit() });
    let system_observer = ctx.system_observer_cap().unwrap_or_else(|| unsafe { thread_exit() });
    let endpoint = ipc_endpoint_create(observability::INTERFACE, observability::VERSION, 16);
    if endpoint == 0 {
        unsafe { thread_exit() };
    }
    let registration = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_REGISTER,
        observability::NAME,
        endpoint,
        IpcRights::SEND | IpcRights::CALL,
    );
    if registration == 0 {
        unsafe { thread_exit() };
    }
    let (status, generation, _) = ipc_reply_wait(registration);
    ipc_close(registration);
    if status != 0 || generation < 1 || ipc_endpoint_bind_cq(endpoint, 0) != 0 {
        unsafe { thread_exit() };
    }

    loop {
        cq_wait(1, 0);
        loop {
            let message = ipc_recv(endpoint);
            if message.status == ipc_status::NO_MESSAGE {
                break;
            }
            if message.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() };
            }
            if !message.is_ok() || message.reply == 0 {
                continue;
            }
            match message.opcode {
                observability::OP_THREAD_SNAPSHOT => {
                    let (memory, length) = thread_statistics_snapshot(system_observer);
                    if memory == 0 || length == 0 {
                        ipc_reply(message.reply, observability::ERR_UNAVAILABLE);
                    } else {
                        ipc_reply_move(message.reply, memory, length as i64);
                    }
                }
                _ => {
                    ipc_reply(message.reply, observability::ERR_BAD_OPCODE);
                }
            }
        }
    }
}
