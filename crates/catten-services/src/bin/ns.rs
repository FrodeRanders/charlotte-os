#![allow(unused_unsafe)]
//! The CharlotteOS userspace name service.
//!
//! Runs in its own EL0 protection domain. The supervisor creates the
//! registry endpoint on this domain's behalf and delivers the endpoint
//! capability through the config-page bootstrap slot.
//!
//! The registry maps service names to `(re-delegable connection, instance
//! generation)`. Names arrive either packed into the scalar argument
//! (up to 8 ASCII bytes) or carried in a copied memory object (up to
//! [`catten_services::MAX_NAME_LEN`] bytes); both transports address the
//! same registry, keyed by the name bytes. Re-registering a name bumps its
//! generation and closes the previous instance's connection, so clients can
//! detect restarts. Lookups return attenuated `SEND | CALL` connections
//! minted from the stored connection — the kernel never sees a name.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    vec::Vec,
};

use catten_rt::{
    Args,
    Input,
    config,
};
use catten_services::{
    MAX_NAME_LEN,
    NAME_SCRATCH_VADDR,
    ns,
};
use catten_syscall::{
    IpcMessage,
    IpcRights,
    ipc_close,
    ipc_recv_block,
    ipc_reply,
    ipc_reply_connection,
    ipc_status,
    memory_close,
    memory_map,
    memory_unmap,
    thread_exit,
};

struct Registration {
    connection: u64,
    generation: i64,
    /// Access key for policy gating: 0 = public (anyone may look up),
    /// non-zero = private (caller must present a matching key via
    /// OP_LOOKUP_KEYED).
    access_key: u64,
}

type Registry = BTreeMap<Vec<u8>, Registration>;

/// Key for a scalar (packed u64) name: the little-endian bytes with
/// trailing NULs trimmed, so `name(b"echo")` and a memory-carried `"echo"`
/// address the same registration.
fn scalar_key(packed: u64) -> Vec<u8> {
    let bytes = packed.to_le_bytes();
    let len = bytes.iter().rposition(|byte| *byte != 0).map_or(0, |index| index + 1);
    bytes[..len].to_vec()
}

/// Read a memory-carried name from the message's copied memory object.
///
/// Always consumes (unmaps and closes) the attached memory cap. Returns
/// `None` for a missing attachment or an empty/oversized length.
fn read_named_key(message: &IpcMessage) -> Option<Vec<u8>> {
    if message.memory == 0 {
        return None;
    }
    let len = message.arg0 as usize;
    if len == 0 || len > MAX_NAME_LEN {
        unsafe {
            memory_close(message.memory);
        }
        return None;
    }
    if unsafe { memory_map(message.memory, NAME_SCRATCH_VADDR, false) } != 0 {
        unsafe {
            memory_close(message.memory);
        }
        return None;
    }
    let mut key = Vec::with_capacity(len);
    unsafe {
        let src = NAME_SCRATCH_VADDR as *const u8;
        for i in 0..len {
            key.push(core::ptr::read_volatile(src.add(i)));
        }
        memory_unmap(message.memory);
        memory_close(message.memory);
    }
    Some(key)
}

fn register(registry: &mut Registry, key: Vec<u8>, connection: u64, access_key: u64) -> i64 {
    let generation = match registry.get(&key) {
        Some(previous) => {
            unsafe {
                ipc_close(previous.connection);
            }
            previous.generation + 1
        }
        None => 1,
    };
    registry.insert(
        key,
        Registration {
            connection,
            generation,
            access_key,
        },
    );
    generation
}

fn reply_lookup(registry: &Registry, key: &[u8], reply: u64, caller_key: u64) {
    match registry.get(key) {
        Some(registration) => {
            // Access check: a registration with access_key=0 is public.
            // A non-zero key must match the caller's presented key.
            if registration.access_key != 0 && registration.access_key != caller_key {
                unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
                return;
            }
            unsafe {
                ipc_reply_connection(
                    reply,
                    registration.connection,
                    IpcRights::SEND | IpcRights::CALL,
                    registration.generation,
                );
            }
        }
        None => unsafe {
            ipc_reply(reply, ns::ERR_NOT_FOUND);
        },
    }
}

fn cmain(_args: Args, _input: Input<0>) -> ! {
    config::write::<u32>(0, 1); // stage: started
    let endpoint = match config::bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(0, 2); // stage: bootstrap endpoint received

    let mut registry: Registry = BTreeMap::new();
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

        match message.opcode {
            ns::OP_REGISTER => {
                let result = if message.connection == 0 {
                    ns::ERR_INVALID
                } else {
                    register(&mut registry, scalar_key(message.arg0), message.connection, 0)
                };
                if message.reply != 0 {
                    unsafe {
                        ipc_reply(message.reply, result);
                    }
                }
            }
            ns::OP_REGISTER_KEYED => {
                let access_key = unsafe { ns::read_access_key(message.memory) };
                let result = if message.connection == 0 {
                    ns::ERR_INVALID
                } else {
                    register(&mut registry, scalar_key(message.arg0), message.connection, access_key)
                };
                if message.reply != 0 {
                    unsafe {
                        ipc_reply(message.reply, result);
                    }
                }
            }
            ns::OP_REGISTER_NAMED => {
                let key = read_named_key(&message);
                let result = match (key, message.connection) {
                    (Some(key), connection) if connection != 0 => {
                        register(&mut registry, key, connection, 0)
                    }
                    (_, connection) => {
                        if connection != 0 {
                            unsafe {
                                ipc_close(connection);
                            }
                        }
                        ns::ERR_INVALID
                    }
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
                reply_lookup(&registry, &scalar_key(message.arg0), message.reply, 0);
            }
            ns::OP_LOOKUP_KEYED => {
                if message.reply == 0 {
                    continue;
                }
                let caller_key = unsafe { ns::read_access_key(message.memory) };
                reply_lookup(&registry, &scalar_key(message.arg0), message.reply, caller_key);
            }
            ns::OP_LOOKUP_NAMED => {
                let key = read_named_key(&message);
                if message.reply == 0 {
                    continue;
                }
                match key {
                    Some(key) => reply_lookup(&registry, &key, message.reply, 0),
                    None => unsafe {
                        ipc_reply(message.reply, ns::ERR_INVALID);
                    },
                }
            }
            _ => {
                if message.memory != 0 {
                    unsafe {
                        memory_close(message.memory);
                    }
                }
                if message.connection != 0 {
                    unsafe {
                        ipc_close(message.connection);
                    }
                }
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
