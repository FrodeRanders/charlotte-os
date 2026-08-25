//! The reference echo service.
//!
//! Creates its own endpoint, registers it with the name service through the
//! bootstrap connection (attaching a re-delegable connection at call time),
//! then serves echo calls **event-driven**: the endpoint's readiness is
//! bound to the domain's default completion queue, and the service blocks on
//! one `CQ_WAIT` — the unified shard wait of the architecture doc §7 — then
//! drains every ready message before waiting again. The same wait would also
//! deliver kernel/device completions and explicit peer wakes.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Context,
    config,
    owned::{
        Endpoint,
        OwnedMemory,
        ReceiveError,
    },
};
use catten_services::{
    echo,
    ns,
    stage_name_owned,
};
use catten_syscall::{
    IpcRights,
    cq_wait,
    thread_exit,
};
use charlotte_launch::echo_status as status;

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1); // stage: started
    let ns_connection = match ctx.bootstrap_connection() {
        Some(connection) => connection,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 2); // stage: bootstrap connection received

    let endpoint = Endpoint::create(echo::INTERFACE, echo::VERSION, 8)
        .unwrap_or_else(|_| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 3); // stage: endpoint created

    // Register under the short (scalar) name.
    let register = ns_connection
        .call_connection(
            ns::OP_REGISTER,
            echo::NAME,
            &endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
        .unwrap_or_else(|_| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 4); // stage: short register call sent
    let generation = register.wait().unwrap_or_else(|_| unsafe { thread_exit() }).result;
    if generation < 1 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::GENERATION, generation as u32);

    // Register the same endpoint under the long (memory-carried) name.
    let name = match stage_name_owned(echo::LONG_NAME) {
        Some(memory) => memory,
        None => unsafe { thread_exit() },
    };
    let register_named = ns_connection
        .call_connection_copy(
            ns::OP_REGISTER_NAMED,
            echo::LONG_NAME.len() as u64,
            &endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
            &name,
        )
        .unwrap_or_else(|_| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 5); // stage: long register call sent
    let named_generation =
        register_named.wait().unwrap_or_else(|_| unsafe { thread_exit() }).result;

    if named_generation < 1 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::NAMED_GENERATION, named_generation as u32);

    // Unified shard wait (§7): bind the endpoint's readiness to the default
    // completion queue, then block on one CQ_WAIT and drain every ready
    // message before waiting again.
    if endpoint.bind_completion_queue(0).is_err() {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 6); // stage: registered and event-driven, serving

    let mut served: u32 = 0;
    let handoff_state = ctx.handoff_state_cap();
    if handoff_state != 0 {
        let memory = unsafe { OwnedMemory::from_raw(handoff_state) }
            .unwrap_or_else(|_| unsafe { thread_exit() });
        let mapping = memory.map_read_only().unwrap_or_else(|_| unsafe { thread_exit() });
        served = u32::from_le_bytes(
            mapping.as_slice()[..4].try_into().unwrap_or_else(|_| unsafe { thread_exit() }),
        );
        config::write::<u32>(status::SERVED, served);
    }

    loop {
        // 1. Block on the single wait point. Releases on endpoint readiness, kernel completions, or
        //    explicit peer wakes alike.
        cq_wait(1, 0);

        // 2. Drain every ready endpoint message. (A full executor would also drain CQ ring entries
        //    and wake tasks here.)
        loop {
            let message = match endpoint.try_receive() {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(ReceiveError::EndpointClosed) => unsafe { thread_exit() },
                Err(_) => break,
            };

            match message.opcode {
                echo::OP_ECHO => {
                    served += 1;
                    config::write::<u32>(status::SERVED, served);
                    if let Some(reply) = message.reply {
                        let _ = reply.reply(message.arg0 as i64);
                    }
                }
                echo::OP_SHUTDOWN => {
                    if let Some(reply) = message.reply {
                        let _ = reply.reply(0);
                    }
                    unsafe { thread_exit() };
                }
                echo::OP_HANDOFF => {
                    // Serialise state: allocate a page, write served count,
                    // move it to the caller (the supervisor).  Reply with
                    // the moved memory cap so the supervisor can hand it
                    // to the replacement service.
                    if let Some(reply) = message.reply {
                        let state = OwnedMemory::allocate(1)
                            .and_then(|memory| memory.map_writable().map_err(|(_, error)| error));
                        match state {
                            Ok(mut mapping) => {
                                mapping.as_mut_slice()[..4].copy_from_slice(&served.to_le_bytes());
                                match mapping.unmap() {
                                    Ok(memory) => {
                                        // Capability ids are address-space-local and must
                                        // not be exported as scalar data.
                                        let _ = reply.reply_move(memory, served as i64);
                                    }
                                    Err(_) => {
                                        let _ = reply.reply(-1);
                                    }
                                }
                            }
                            Err(_) => {
                                let _ = reply.reply(-1);
                            }
                        }
                    }
                    unsafe { thread_exit() };
                }
                _ => {
                    if let Some(reply) = message.reply {
                        let _ = reply.reply(-1);
                    }
                }
            }
        }
    }
}

catten_rt::entry!(main);
