//! CharlotteOS owned-snapshot observability service.
//!
//! This first version exposes scheduler statistics for its own address space
//! through endpoint IPC. It deliberately cannot inspect other domains:
//! services retain ownership of their telemetry until they explicitly publish
//! it or receive a future delegated system-observer capability.
#![no_std]
#![no_main]

catten_rt::entry!(main);

use catten_rt::{
    Context,
    owned::{
        Endpoint,
        OwnedMemory,
        ReceiveError,
    },
};
use catten_services::{
    ns,
    observability,
};
use catten_syscall::{
    IpcRights,
    cq_wait,
    thread_exit,
    thread_statistics_snapshot,
};

fn main(ctx: Context) -> ! {
    let ns_connection = ctx.bootstrap_connection().unwrap_or_else(|| unsafe { thread_exit() });
    let system_observer = ctx.system_observer_cap().unwrap_or_else(|| unsafe { thread_exit() });
    let endpoint = Endpoint::create(observability::INTERFACE, observability::VERSION, 16)
        .unwrap_or_else(|_| unsafe { thread_exit() });
    let generation = ns_connection
        .call_connection(
            ns::OP_REGISTER,
            observability::NAME,
            &endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
        .and_then(|call| call.wait())
        .unwrap_or_else(|_| unsafe { thread_exit() })
        .result;
    if generation < 1 || endpoint.bind_completion_queue(0).is_err() {
        unsafe { thread_exit() };
    }

    loop {
        cq_wait(1, 0);
        loop {
            let message = match endpoint.try_receive() {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(ReceiveError::EndpointClosed) => unsafe { thread_exit() },
                Err(_) => continue,
            };
            let Some(reply) = message.reply else {
                continue;
            };
            match message.opcode {
                observability::OP_THREAD_SNAPSHOT => {
                    let (memory, length) = thread_statistics_snapshot(system_observer);
                    if memory == 0 || length == 0 {
                        let _ = reply.reply(observability::ERR_UNAVAILABLE);
                    } else {
                        // thread_statistics_snapshot creates a new owned
                        // memory capability for this caller.
                        match unsafe { OwnedMemory::from_raw(memory) } {
                            Ok(memory) => {
                                let _ = reply.reply_move(memory, length as i64);
                            }
                            Err(_) => {
                                let _ = reply.reply(observability::ERR_UNAVAILABLE);
                            }
                        }
                    }
                }
                _ => {
                    let _ = reply.reply(observability::ERR_BAD_OPCODE);
                }
            }
        }
    }
}
