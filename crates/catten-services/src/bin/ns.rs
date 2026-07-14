//! The CharlotteOS userspace name service.
//!
//! Runs in its own EL0 protection domain. The supervisor creates the
//! registry endpoint on this domain's behalf and delivers the endpoint
//! capability through the config-page bootstrap slot.
//!
//! The registry maps packed u64 names to `(re-delegable connection,
//! instance generation)`. Re-registering a name bumps its generation and
//! closes the previous instance's connection, so clients can detect
//! restarts. Lookups return attenuated `SEND | CALL` connections minted
//! from the stored connection — the kernel never sees a name.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;

use catten_rt::{
    Args,
    Input,
    config,
};
use catten_services::ns;
use catten_syscall::{
    IpcRights,
    ipc_close,
    ipc_recv_block,
    ipc_reply,
    ipc_reply_connection,
    ipc_status,
    thread_exit,
};

struct Registration {
    connection: u64,
    generation: i64,
}

fn cmain(_args: Args, _input: Input<0>) -> ! {
    config::write::<u32>(0, 1); // stage: started
    let endpoint = match config::bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(0, 2); // stage: bootstrap endpoint received

    let mut registry: BTreeMap<u64, Registration> = BTreeMap::new();
    let mut handled: u32 = 0;

    loop {
        let message = unsafe { ipc_recv_block(endpoint) };
        if message.status == ipc_status::ENDPOINT_CLOSED {
            unsafe { thread_exit() };
        }
        if !message.is_ok() {
            continue;
        }
        handled += 1;
        config::write::<u32>(4, handled);
        config::write::<u32>(8, message.opcode);
        handled += 1;
        config::write::<u32>(4, handled);
        config::write::<u32>(8, message.opcode);

        match message.opcode {
            ns::OP_REGISTER => {
                let result = if message.connection == 0 {
                    ns::ERR_INVALID
                } else {
                    let generation = match registry.get(&message.arg0) {
                        Some(previous) => {
                            unsafe {
                                ipc_close(previous.connection);
                            }
                            previous.generation + 1
                        }
                        None => 1,
                    };
                    registry.insert(
                        message.arg0,
                        Registration {
                            connection: message.connection,
                            generation,
                        },
                    );
                    generation
                };
                if message.reply != 0 {
                    unsafe {
                        ipc_reply(message.reply, result);
                    }
                }
            }
            ns::OP_LOOKUP => {
                if message.reply == 0 {
                    continue;
                }
                match registry.get(&message.arg0) {
                    Some(registration) => unsafe {
                        ipc_reply_connection(
                            message.reply,
                            registration.connection,
                            IpcRights::SEND | IpcRights::CALL,
                            registration.generation,
                        );
                    },
                    None => unsafe {
                        ipc_reply(message.reply, ns::ERR_NOT_FOUND);
                    },
                }
            }
            _ => {
                if message.reply != 0 {
                    unsafe {
                        ipc_reply(message.reply, ns::ERR_BAD_OPCODE);
                    }
                }
            }
        }
    }
}

catten_rt::entry!(cmain);
