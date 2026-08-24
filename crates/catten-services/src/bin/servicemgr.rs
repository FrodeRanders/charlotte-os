//! The CharlotteOS service manager — orchestrates live service upgrades.
//!
//! Bootstraps, registers as "svcmgr", and serves OP_UPGRADE requests.
//! An upgrade drains the target service, receives its handoff state
//! (memory object), then invokes the capability-checked kernel upgrade
//! operation to load and start the replacement.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    authorization::{
        AuthorizationRights,
        wire,
    },
    echo,
    ns,
    objstore,
};
use catten_syscall::{
    IpcRights,
    cq_wait,
    ipc_close,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_wait,
    ipc_reply_wait_with_memory,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_scalar_call_copy,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
    spawn_upgrade,
    thread_exit,
};
use charlotte_launch::service_manager_status as status;

/// Pack 6 ASCII bytes into a u64 name.
const fn name(s: &[u8]) -> u64 {
    let mut packed = [0u8; 8];
    let mut i = 0;
    while i < s.len() && i < 8 {
        packed[i] = s[i];
        i += 1;
    }
    u64::from_le_bytes(packed)
}

const MGR_NAME: u64 = name(b"svcmgr");

/// Block until a pending call completes. Returns `(result, returned_cap)`.
unsafe fn spin_call(call: u64, _what: &str) -> (u64, u64) {
    let (status, result, cap) = ipc_reply_wait(call);
    ipc_close(call);
    if status == 0 {
        (result, cap)
    } else {
        unsafe { thread_exit() }
    }
}

/// Look up a service by short name; return its connection cap.
fn lookup(ns_conn: u64, target: u64) -> Option<(u64, u64)> {
    let l = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, target);
    if l == 0 {
        return None;
    }
    let (status, generation, cap) = ipc_reply_wait(l);
    ipc_close(l);
    if status == 0 && generation >= 1 && cap != 0 {
        Some((generation, cap))
    } else {
        None
    }
}

fn policy_call(ns_conn: u64, opcode: u32, request: &[u8]) -> Option<(i64, u64)> {
    let memory = memory_alloc(1);
    if memory == 0 {
        return None;
    }
    let (status, base) = memory_map_any(memory, true);
    if status != 0 {
        memory_close(memory);
        return None;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(request.as_ptr(), base as *mut u8, request.len());
    }
    memory_unmap(memory);
    let call = ipc_scalar_call_copy(ns_conn, opcode, request.len() as u64, memory);
    memory_close(memory);
    if call == 0 {
        return None;
    }
    let (status, result, connection) = ipc_reply_wait(call);
    ipc_close(call);
    (status == 0).then_some((result as i64, connection))
}

fn verify_authorized_lookup(ns_conn: u64) -> bool {
    let identity = catten_syscall::get_domain_identity();
    if identity.principal == 0 || identity.roles & catten_syscall::domain_roles::POLICY_ADMIN == 0 {
        return false;
    }
    let mut request = [0u8; wire::MAX_REQUEST_LEN];
    let Some(length) = wire::encode_set_policy(
        b"echo",
        identity.principal,
        AuthorizationRights::CALL,
        0,
        &mut request,
    ) else {
        return false;
    };
    if let Some((_version, returned)) = policy_call(ns_conn, ns::OP_SET_POLICY, &request[..length])
        && returned != 0
    {
        ipc_close(returned);
    }

    let Some(length) = wire::encode_lookup(b"echo", AuthorizationRights::CALL, &mut request) else {
        return false;
    };
    let Some((generation, connection)) =
        policy_call(ns_conn, ns::OP_LOOKUP_AUTHORIZED, &request[..length])
    else {
        return false;
    };
    if generation < 1 || connection == 0 {
        return false;
    }
    ipc_close(connection);
    true
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let ns_connection = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 2);

    let ep = ipc_endpoint_create(0x5356434d, 1, 8); // "SVCM"
    if ep == 0 {
        unsafe { thread_exit() };
    }
    let reg = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_REGISTER,
        MGR_NAME,
        ep,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if reg == 0 {
        unsafe { thread_exit() };
    }
    let r = unsafe { spin_call(reg, "register") };
    if r.0 == 0 {
        unsafe { thread_exit() };
    }

    if !verify_authorized_lookup(ns_connection) {
        config::write::<u32>(status::ERROR, 8);
        unsafe { thread_exit() };
    }

    if ipc_endpoint_bind_cq(ep, 0) != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 3);

    loop {
        cq_wait(1, 0);
        loop {
            let m = ipc_recv(ep);
            if m.status == ipc_status::NO_MESSAGE {
                break;
            }
            if m.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() };
            }
            if !m.is_ok() {
                break;
            }

            if m.opcode == 1 && m.reply != 0 {
                config::write::<u32>(status::STAGE, 10);
                let result = do_upgrade(ns_connection, m.arg0);
                config::write::<u32>(status::LAST_GENERATION, result as u32);
                ipc_reply(m.reply, result);
            } else if m.reply != 0 {
                ipc_reply(m.reply, -1);
            }
        }
    }
}

/// Orchestrate the handoff and return the result code.
fn do_upgrade(ns_conn: u64, target_name: u64) -> i64 {
    let (old_generation, target_conn) = match lookup(ns_conn, target_name) {
        Some(found) => found,
        None => {
            config::write::<u32>(status::ERROR, 1);
            return -1;
        }
    };
    config::write::<u32>(status::STAGE, 11);

    // OP_HANDOFF: the target serialises state, returns (state_cap, ep_cap),
    // and exits. The wait returns the moved memory cap without polling.
    let call = ipc_scalar_call(target_conn, echo::OP_HANDOFF, 0);
    if call == 0 {
        config::write::<u32>(status::ERROR, 2);
        return -2;
    }

    let (status, _handoff_result, _conn, state_cap) = ipc_reply_wait_with_memory(call);
    catten_syscall::ipc_close(call);
    if status != 0 {
        config::write::<u32>(status::ERROR, 3);
        return -3;
    }
    config::write::<u32>(status::STAGE, 12);

    if state_cap == 0 {
        config::write::<u32>(status::ERROR, 4);
        return -4;
    }

    // Remove the old generation from discovery before starting the
    // replacement. Ordinary OP_LOOKUP calls now defer in the name service
    // until OP_REGISTER publishes the next generation, so no special
    // completion path is needed for the upgrade transaction.
    let unregister = ipc_scalar_call(ns_conn, ns::OP_UNREGISTER, target_name);
    if unregister == 0 {
        config::write::<u32>(status::ERROR, 6);
        return -6;
    }
    let (unregister_status, unpublished_generation, _) = ipc_reply_wait(unregister);
    ipc_close(unregister);
    if unregister_status != 0 || unpublished_generation != old_generation {
        config::write::<u32>(status::ERROR, 6);
        return -6;
    }

    // Record the handoff state for diagnostics, then ask the kernel
    // supervisor to move it into and start the replacement echo image.
    config::write::<u64>(status::STATE_CAPABILITY, state_cap);
    config::write::<u64>(status::ENDPOINT_CAPABILITY, target_conn);
    config::write::<u32>(status::STAGE, 4);
    let (elf_cap, elf_size) = persistent_replacement(ns_conn);
    let replacement_asid = unsafe { spawn_upgrade(elf_cap, elf_size, state_cap, target_conn) };
    if replacement_asid == 0 {
        config::write::<u32>(status::ERROR, 5);
        return -5;
    }
    config::write::<u32>(status::STAGE, 13);

    // OP_UNREGISTER left a tombstone, so this ordinary lookup is either
    // satisfied immediately by generation N+1 or retained by the name
    // service until that registration occurs. The completion is therefore
    // the single authoritative publication event for the upgrade.
    let (new_generation, replacement_connection) = match lookup(ns_conn, target_name) {
        Some(found) => found,
        None => {
            config::write::<u32>(status::ERROR, 7);
            return -7;
        }
    };
    if new_generation <= old_generation {
        ipc_close(replacement_connection);
        config::write::<u32>(status::ERROR, 7);
        return -7;
    }
    ipc_close(replacement_connection);
    config::write::<u32>(status::STAGE, 5);
    replacement_asid as i64
}

/// Fetch the installed replacement image when persistent storage is online.
///
/// Returning `(0, 0)` selects the embedded echo image as a bootstrap/recovery
/// fallback. A malformed stored ELF is rejected by the kernel loader rather
/// than silently falling back, so a corrupt or unauthenticated update cannot
/// masquerade as a successful installation.
fn persistent_replacement(ns_conn: u64) -> (u64, u64) {
    let lookup = ipc_scalar_call_connection(
        ns_conn,
        ns::OP_TRY_LOOKUP,
        objstore::NAME,
        0,
        IpcRights::SEND | IpcRights::CALL,
    );
    if lookup == 0 {
        return (0, 0);
    }
    let (status, _generation, object_connection) = ipc_reply_wait(lookup);
    ipc_close(lookup);
    if status != 0 || object_connection == 0 {
        return (0, 0);
    }
    let read = ipc_scalar_call(object_connection, objstore::OP_READ, objstore::EXECUTABLE_ECHO_ID);
    ipc_close(object_connection);
    if read == 0 {
        return (0, 0);
    }
    let (read_status, size, returned_connection, memory) = ipc_reply_wait_with_memory(read);
    ipc_close(read);
    if returned_connection != 0 {
        ipc_close(returned_connection);
    }
    if read_status != 0 || memory == 0 || size == 0 {
        if memory != 0 {
            catten_syscall::memory_close(memory);
        }
        (0, 0)
    } else {
        (memory, size)
    }
}

catten_rt::entry!(main);
