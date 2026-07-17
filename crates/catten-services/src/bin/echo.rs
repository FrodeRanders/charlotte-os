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
    Args,
    Input,
    config,
};
use catten_services::{
    echo,
    ns,
    stage_name,
    wait_reply,
};
use catten_syscall::{
    IpcRights,
    cq_wait,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_scalar_call_connection,
    ipc_scalar_call_connection_copy,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_get_phys,
    memory_map,
    memory_unmap,
    thread_exit,
};

const REPLY_SPINS: u64 = 50_000_000;

fn cmain(_args: Args, _input: Input<0>) -> ! {
    config::write::<u32>(0, 1); // stage: started
    let ns_connection = match config::bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(0, 2); // stage: bootstrap connection received

    let endpoint = unsafe { ipc_endpoint_create(echo::INTERFACE, echo::VERSION, 8) };
    if endpoint == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 3); // stage: endpoint created

    // Register under the short (scalar) name.
    let register = unsafe {
        ipc_scalar_call_connection(
            ns_connection,
            ns::OP_REGISTER,
            echo::NAME,
            endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
    };
    if register == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 4); // stage: short register call sent
    let (generation, _) = unsafe { wait_reply(register, REPLY_SPINS) };
    if generation < 1 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(4, generation as u32);

    // Register the same endpoint under the long (memory-carried) name.
    let name_cap = match unsafe { stage_name(echo::LONG_NAME) } {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let register_named = unsafe {
        ipc_scalar_call_connection_copy(
            ns_connection,
            ns::OP_REGISTER_NAMED,
            echo::LONG_NAME.len() as u64,
            endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
            name_cap,
        )
    };
    if register_named == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 5); // stage: long register call sent
    let (named_generation, _) = unsafe { wait_reply(register_named, REPLY_SPINS) };
    unsafe {
        memory_close(name_cap);
    }
    if named_generation < 1 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(12, named_generation as u32);

    // Unified shard wait (§7): bind the endpoint's readiness to the default
    // completion queue, then block on one CQ_WAIT and drain every ready
    // message before waiting again.
    if unsafe { ipc_endpoint_bind_cq(endpoint, 0) } != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 6); // stage: registered and event-driven, serving

    let mut served: u32 = 0;

    loop {
        // 1. Block on the single wait point. Releases on endpoint readiness,
        //    kernel completions, or explicit peer wakes alike.
        unsafe {
            cq_wait(1, 0);
        }

        // 2. Drain every ready endpoint message. (A full executor would also
        //    drain CQ ring entries and wake tasks here.)
        loop {
            let message = unsafe { ipc_recv(endpoint) };
            if message.status == ipc_status::NO_MESSAGE {
                break;
            }
            if message.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() };
            }
            if !message.is_ok() {
                break;
            }

            match message.opcode {
                echo::OP_ECHO => {
                    served += 1;
                    config::write::<u32>(8, served);
                    if message.reply != 0 {
                        unsafe {
                            ipc_reply(message.reply, message.arg0 as i64);
                        }
                    }
                }
                echo::OP_SHUTDOWN => {
                    if message.reply != 0 {
                        unsafe {
                            ipc_reply(message.reply, 0);
                        }
                    }
                    unsafe { thread_exit() };
                }
                echo::OP_HANDOFF => {
                    // Serialise state: allocate a page, write served count,
                    // move it to the caller (the supervisor).  Reply with
                    // the moved memory cap so the supervisor can hand it
                    // to the replacement service.
                    let state_cap = unsafe { memory_alloc(1) };
                    if state_cap != 0 {
                        // Use HEAP_VADDR + high offset as scratch (above the
                        // long-name scratch at 0x100000).
                        const STATE_VADDR: usize = 0x0000_0000_00a0_0000;
                        if unsafe { memory_map(state_cap, STATE_VADDR, true) } == 0 {
                            unsafe {
                                core::ptr::write_volatile(
                                    STATE_VADDR as *mut u32, served,
                                );
                                memory_unmap(state_cap);
                            }
                        }
                        if message.reply != 0 {
                            unsafe {
                                ipc_reply_move(message.reply, state_cap, served as i64);
                            }
                        }
                    } else if message.reply != 0 {
                        unsafe { ipc_reply(message.reply, -1) };
                    }
                    unsafe { thread_exit() };
                }
                _ => {
                    if message.reply != 0 {
                        unsafe {
                            ipc_reply(message.reply, -1);
                        }
                    }
                }
            }
        }
    }
}

catten_rt::entry!(cmain);
