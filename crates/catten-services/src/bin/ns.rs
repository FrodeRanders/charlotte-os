#![allow(unused_unsafe)]
//! The CharlotteOS userspace name service.
//!
//! Runs in its own EL0 protection domain. Maps service names to
//! `(re-delegable connection, instance generation)`.
//!
//! **Deferred lookups:** if OP_LOOKUP arrives before the service registers,
//! the name service retains the reply token. When the service later calls
//! OP_REGISTER, all waiting callers receive their connections. No polling,
//! no retry loops — the caller's future resolves when the service appears.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    vec::Vec,
};

use catten_rt::{
    Context,
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
    access_key: u64,
}

type Registry = BTreeMap<Vec<u8>, Registration>;
/// Deferred lookups: name → list of reply tokens waiting for registration.
type Waitlist = BTreeMap<Vec<u8>, Vec<u64>>;

fn scalar_key(packed: u64) -> Vec<u8> {
    let bytes = packed.to_le_bytes();
    let len = bytes.iter().rposition(|byte| *byte != 0).map_or(0, |index| index + 1);
    bytes[..len].to_vec()
}

fn read_named_key(message: &IpcMessage) -> Option<Vec<u8>> {
    if message.memory == 0 {
        return None;
    }
    let len = message.arg0 as usize;
    if len == 0 || len > MAX_NAME_LEN {
        unsafe { memory_close(message.memory); }
        return None;
    }
    if unsafe { memory_map(message.memory, NAME_SCRATCH_VADDR, false) } != 0 {
        unsafe { memory_close(message.memory); }
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

fn register(
    registry: &mut Registry,
    waitlist: &mut Waitlist,
    key: Vec<u8>,
    connection: u64,
    access_key: u64,
) -> i64 {
    let generation = match registry.get(&key) {
        Some(previous) => {
            if previous.connection != 0 {
                unsafe { ipc_close(previous.connection); }
            }
            previous.generation + 1
        }
        None => 1,
    };
    registry.insert(
        key.clone(),
        Registration { connection, generation, access_key },
    );

    // Wake all callers waiting for this service.
    if let Some(waiters) = waitlist.remove(&key) {
        for reply in waiters {
            unsafe {
                ipc_reply_connection(
                    reply,
                    connection,
                    IpcRights::SEND | IpcRights::CALL,
                    generation,
                );
            }
        }
    }
    generation
}

fn lookup_or_defer(
    registry: &Registry,
    waitlist: &mut Waitlist,
    key: &[u8],
    reply: u64,
    caller_key: u64,
) {
    match registry.get(key) {
        Some(registration) if registration.connection != 0 => {
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
        _ => {
            // Defer: retain the reply token until the service registers.
            waitlist.entry(key.to_vec()).or_default().push(reply);
        }
    }
}

fn try_lookup(registry: &Registry, key: &[u8], reply: u64) {
    match registry.get(key) {
        Some(registration) if registration.connection != 0 => unsafe {
            ipc_reply_connection(
                reply,
                registration.connection,
                IpcRights::SEND | IpcRights::CALL,
                registration.generation,
            );
        },
        _ => unsafe {
            ipc_reply(reply, ns::ERR_NOT_FOUND);
        },
    }
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(0, 1);
    let endpoint = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(0, 2);

    let mut registry: Registry = BTreeMap::new();
    let mut waitlist: Waitlist = BTreeMap::new();
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
        config::write::<u32>(12, waitlist.len() as u32);

        match message.opcode {
            ns::OP_REGISTER => {
                let result = if message.connection == 0 {
                    ns::ERR_INVALID
                } else {
                    register(
                        &mut registry,
                        &mut waitlist,
                        scalar_key(message.arg0),
                        message.connection,
                        0,
                    )
                };
                if message.reply != 0 {
                    unsafe { ipc_reply(message.reply, result); }
                }
            }
            ns::OP_REGISTER_KEYED => {
                let access_key = unsafe { ns::read_access_key(message.memory) };
                let result = if message.connection == 0 {
                    ns::ERR_INVALID
                } else {
                    register(
                        &mut registry,
                        &mut waitlist,
                        scalar_key(message.arg0),
                        message.connection,
                        access_key,
                    )
                };
                if message.reply != 0 {
                    unsafe { ipc_reply(message.reply, result); }
                }
            }
            ns::OP_REGISTER_NAMED => {
                let key = read_named_key(&message);
                let result = match (key, message.connection) {
                    (Some(key), connection) if connection != 0 => {
                        register(
                            &mut registry,
                            &mut waitlist,
                            key,
                            connection,
                            0,
                        )
                    }
                    (_, connection) => {
                        if connection != 0 {
                            unsafe { ipc_close(connection); }
                        }
                        ns::ERR_INVALID
                    }
                };
                if message.reply != 0 {
                    unsafe { ipc_reply(message.reply, result); }
                }
            }
            ns::OP_LOOKUP => {
                if message.reply == 0 { continue; }
                lookup_or_defer(
                    &registry,
                    &mut waitlist,
                    &scalar_key(message.arg0),
                    message.reply,
                    0,
                );
            }
            ns::OP_UNREGISTER => {
                let key = scalar_key(message.arg0);
                let result = match registry.get_mut(&key) {
                    Some(registration) if registration.connection != 0 => {
                        unsafe { ipc_close(registration.connection); }
                        registration.connection = 0;
                        registration.generation
                    }
                    _ => ns::ERR_NOT_FOUND,
                };
                if message.reply != 0 {
                    unsafe { ipc_reply(message.reply, result); }
                }
            }
            ns::OP_TRY_LOOKUP => {
                if message.reply != 0 {
                    try_lookup(&registry, &scalar_key(message.arg0), message.reply);
                }
            }
            ns::OP_LOOKUP_KEYED => {
                if message.reply == 0 { continue; }
                let caller_key = unsafe { ns::read_access_key(message.memory) };
                lookup_or_defer(
                    &registry,
                    &mut waitlist,
                    &scalar_key(message.arg0),
                    message.reply,
                    caller_key,
                );
            }
            ns::OP_LOOKUP_NAMED => {
                let key = read_named_key(&message);
                if message.reply == 0 { continue; }
                match key {
                    Some(key) => lookup_or_defer(&registry, &mut waitlist, &key, message.reply, 0),
                    None => unsafe { ipc_reply(message.reply, ns::ERR_INVALID); },
                }
            }
            _ => {
                if message.memory != 0 { unsafe { memory_close(message.memory); } }
                if message.connection != 0 { unsafe { ipc_close(message.connection); } }
                if message.reply != 0 { unsafe { ipc_reply(message.reply, ns::ERR_BAD_OPCODE); } }
            }
        }
    }
}

catten_rt::entry!(main);
