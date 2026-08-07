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
    broker::EventBroker,
    ns,
};
use catten_syscall::{
    IpcMessage,
    IpcRights,
    ipc_close,
    ipc_recv_block,
    ipc_reply,
    ipc_reply_connection,
    ipc_reply_move,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
    thread_exit,
};

const STATUS_SNAPSHOT_MAX: usize = 4096;

struct Registration {
    connection: u64,
    generation: i64,
    access_key: u64,
}

type Registry = BTreeMap<Vec<u8>, Registration>;

/// The registry viewed as an immediate catalog (the broker's lookups).
struct RegistryCatalog<'a>(&'a Registry);

impl catten_services::broker::Catalog for RegistryCatalog<'_> {
    fn resolve(&self, name: &[u8]) -> Option<catten_services::broker::CatalogTarget> {
        // The unregister tombstone (connection == 0) is not a live
        // registration: resolving it would make KeyedWaitlist::park return
        // the waiter instead of parking it, and the lookup path would then
        // discard the reply token (a lost reply and a forever-stalled
        // caller).
        self.0.get(name).and_then(|registration| {
            (registration.connection != 0).then(|| catten_services::broker::CatalogTarget {
                generation: registration.generation as u64,
                connection: registration.connection,
            })
        })
    }
}
/// Deferred lookups: name → reply token and the access key supplied by its
/// caller. Retaining the key is necessary because registration may establish
/// an access policy after the lookup has blocked. The waitlist is the
/// service's *event-broker* face; the registry is its *catalog* face (see
/// `catten_services::broker`).
type Waitlist = catten_services::broker::KeyedWaitlist<(u64, u64)>;



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
        unsafe {
            memory_close(message.memory);
        }
        return None;
    }
        let (name_scratch_vaddr_0_map_status, name_scratch_vaddr_0) = memory_map_any(message.memory, false);
    if unsafe { name_scratch_vaddr_0_map_status } != 0 {
        unsafe {
            memory_close(message.memory);
        }
        return None;
    }
    let mut key = Vec::with_capacity(len);
    unsafe {
        let src = name_scratch_vaddr_0 as *const u8;
        for i in 0..len {
            key.push(core::ptr::read_volatile(src.add(i)));
        }
        memory_unmap(message.memory);
        memory_close(message.memory);
    }
    Some(key)
}

fn read_generation(message: &IpcMessage) -> Option<u64> {
    if message.memory == 0 {
        return None;
    }
        let (name_scratch_vaddr_1_map_status, name_scratch_vaddr_1) = memory_map_any(message.memory, false);
    if unsafe { name_scratch_vaddr_1_map_status } != 0 {
        unsafe { memory_close(message.memory) };
        return None;
    }
    let generation = unsafe { core::ptr::read_volatile(name_scratch_vaddr_1 as *const u64) };
    unsafe {
        memory_unmap(message.memory);
        memory_close(message.memory);
    }
    Some(generation)
}

fn register(
    registry: &mut Registry,
    waitlist: &mut Waitlist,
    key: Vec<u8>,
    connection: u64,
    access_key: u64,
) -> i64 {
    let generation = match registry.get(&key) {
        Some(previous) => previous.generation + 1,
        None => 1,
    };
    // Publishing the new entry is the replacement linearization point. Retire
    // the old connection only after no subsequent lookup can observe it.
    let previous = registry.insert(
        key.clone(),
        Registration {
            connection,
            generation,
            access_key,
        },
    );
    if let Some(previous) = previous
        && previous.connection != 0
    {
        unsafe {
            ipc_close(previous.connection);
        }
    }

    // Wake all callers only after the new generation is authoritative.
    for (reply, caller_key) in waitlist.fire(&key) {
        if access_key != 0 && access_key != caller_key {
            unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
        } else {
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
            // Defer: the event broker retains the reply token until the
            // service registers (fulfillment by the publishing side).
            let _ = waitlist.park(key, (reply, caller_key), &RegistryCatalog(registry));
        }
    }
}

fn try_lookup(registry: &Registry, key: &[u8], reply: u64) {
    match registry.get(key) {
        Some(registration) if registration.connection != 0 && registration.access_key == 0 => unsafe {
            ipc_reply_connection(
                reply,
                registration.connection,
                IpcRights::SEND | IpcRights::CALL,
                registration.generation,
            );
        },
        Some(registration) if registration.connection != 0 => unsafe {
            ipc_reply(reply, ns::ERR_ACCESS_DENIED);
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
    let mut waitlist: Waitlist = catten_services::broker::KeyedWaitlist::new();
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
                    register(
                        &mut registry,
                        &mut waitlist,
                        scalar_key(message.arg0),
                        message.connection,
                        access_key,
                    )
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
                        register(&mut registry, &mut waitlist, key, connection, 0)
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
                        unsafe {
                            ipc_close(registration.connection);
                        }
                        registration.connection = 0;
                        registration.generation
                    }
                    _ => ns::ERR_NOT_FOUND,
                };
                if message.reply != 0 {
                    unsafe {
                        ipc_reply(message.reply, result);
                    }
                }
            }
            ns::OP_UNREGISTER_GENERATION => {
                let key = scalar_key(message.arg0);
                let expected_generation = read_generation(&message);
                let result = match (registry.get_mut(&key), expected_generation) {
                    (Some(registration), Some(expected))
                        if registration.connection != 0
                            && u64::try_from(registration.generation) == Ok(expected) =>
                    {
                        unsafe {
                            ipc_close(registration.connection);
                        }
                        registration.connection = 0;
                        registration.generation
                    }
                    _ => ns::ERR_NOT_FOUND,
                };
                if message.reply != 0 {
                    unsafe {
                        ipc_reply(message.reply, result);
                    }
                }
            }
            ns::OP_TRY_LOOKUP => {
                if message.reply != 0 {
                    try_lookup(&registry, &scalar_key(message.arg0), message.reply);
                }
            }
            ns::OP_LOOKUP_KEYED => {
                if message.reply == 0 {
                    continue;
                }
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
                if message.reply == 0 {
                    continue;
                }
                match key {
                    Some(key) => lookup_or_defer(&registry, &mut waitlist, &key, message.reply, 0),
                    None => unsafe {
                        ipc_reply(message.reply, ns::ERR_INVALID);
                    },
                }
            }
            ns::OP_STATUS => {
                let cap = memory_alloc(1);
                if cap == 0 {
                    if message.reply != 0 {
                        unsafe { ipc_reply(message.reply, ns::ERR_BAD_OPCODE) };
                    }
                    continue;
                }
        let (status_scratch_map_status, status_scratch_vaddr) = memory_map_any(cap, true);
                if status_scratch_map_status != 0 {
                    memory_close(cap);
                    if message.reply != 0 {
                        unsafe { ipc_reply(message.reply, ns::ERR_BAD_OPCODE) };
                    }
                    continue;
                }
                let mut length = 0usize;
                unsafe {
                    core::ptr::write_volatile(
                        (status_scratch_vaddr + ns::STATUS_OFFSET_MAGIC as usize * 4) as *mut u32,
                        ns::STATUS_MAGIC,
                    );
                    core::ptr::write_volatile(
                        (status_scratch_vaddr + ns::STATUS_OFFSET_REGISTERED as usize * 4) as *mut u32,
                        registry.len() as u32,
                    );
                    core::ptr::write_volatile(
                        (status_scratch_vaddr + ns::STATUS_OFFSET_PENDING as usize * 4) as *mut u32,
                        waitlist.len() as u32,
                    );
                }
                length += 12;
                for key in registry.keys() {
                    let name_len = key.len().min(255);
                    if length + 1 + name_len > STATUS_SNAPSHOT_MAX {
                        break;
                    }
                    unsafe {
                        core::ptr::write_volatile(
                            (status_scratch_vaddr + length) as *mut u8,
                            name_len as u8,
                        );
                        core::ptr::copy_nonoverlapping(
                            key.as_ptr(),
                            (status_scratch_vaddr + length + 1) as *mut u8,
                            name_len,
                        );
                    }
                    length += 1 + name_len;
                }
                memory_unmap(cap);
                if message.reply != 0 {
                    unsafe { ipc_reply_move(message.reply, cap, length as i64) };
                } else {
                    memory_close(cap);
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

catten_rt::entry!(main);
