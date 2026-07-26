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
    echo,
    ns,
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
    ipc_status,
    spawn_upgrade,
    thread_exit,
};

const STAGE_OFFSET: usize = 0;
const LAST_GEN_OFFSET: usize = 4;
const ERROR_OFFSET: usize = 8;
const STATE_CAP_OFFSET: usize = 16;
const ENDPOINT_CAP_OFFSET: usize = 24;

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

fn main(ctx: Context) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1);
    let ns_connection = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(STAGE_OFFSET, 2);

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

    if ipc_endpoint_bind_cq(ep, 0) != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 3);

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
                config::write::<u32>(STAGE_OFFSET, 10);
                let result = do_upgrade(ns_connection, m.arg0);
                config::write::<u32>(LAST_GEN_OFFSET, result as u32);
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
            config::write::<u32>(ERROR_OFFSET, 1);
            return -1;
        }
    };
    config::write::<u32>(STAGE_OFFSET, 11);

    // Arm the generation barrier before asking the old service to hand off.
    // OP_LOOKUP_NEXT deliberately ignores the current registration and is
    // completed by the replacement's later OP_REGISTER. Arming first makes
    // registration-before-wait safe without polling the name service.
    let next_registration = ipc_scalar_call(ns_conn, ns::OP_LOOKUP_NEXT, target_name);
    if next_registration == 0 {
        config::write::<u32>(ERROR_OFFSET, 6);
        return -6;
    }
    let barrier = ipc_scalar_call(ns_conn, ns::OP_BARRIER, 0);
    if barrier == 0 {
        ipc_close(next_registration);
        config::write::<u32>(ERROR_OFFSET, 8);
        return -8;
    }
    let (barrier_status, barrier_result, _) = ipc_reply_wait(barrier);
    ipc_close(barrier);
    if barrier_status != 0 || barrier_result != 0 {
        ipc_close(next_registration);
        config::write::<u32>(ERROR_OFFSET, 8);
        return -8;
    }

    // OP_HANDOFF: the target serialises state, returns (state_cap, ep_cap),
    // and exits. The wait returns the moved memory cap without polling.
    let call = ipc_scalar_call(target_conn, echo::OP_HANDOFF, 0);
    if call == 0 {
        ipc_close(next_registration);
        config::write::<u32>(ERROR_OFFSET, 2);
        return -2;
    }

    let (status, _handoff_result, _conn, state_cap) = ipc_reply_wait_with_memory(call);
    catten_syscall::ipc_close(call);
    if status != 0 {
        ipc_close(next_registration);
        config::write::<u32>(ERROR_OFFSET, 3);
        return -3;
    }
    config::write::<u32>(STAGE_OFFSET, 12);

    if state_cap == 0 {
        ipc_close(next_registration);
        config::write::<u32>(ERROR_OFFSET, 4);
        return -4;
    }

    // Record the handoff state for diagnostics, then ask the kernel
    // supervisor to move it into and start the replacement echo image.
    config::write::<u64>(STATE_CAP_OFFSET, state_cap);
    config::write::<u64>(ENDPOINT_CAP_OFFSET, target_conn);
    config::write::<u32>(STAGE_OFFSET, 4);
    let replacement_asid = unsafe { spawn_upgrade(0, state_cap, target_conn) };
    if replacement_asid == 0 {
        ipc_close(next_registration);
        config::write::<u32>(ERROR_OFFSET, 5);
        return -5;
    }

    let (status, new_generation, replacement_connection) = ipc_reply_wait(next_registration);
    ipc_close(next_registration);
    if status != 0
        || new_generation <= old_generation
        || replacement_connection == 0
    {
        if replacement_connection != 0 {
            ipc_close(replacement_connection);
        }
        config::write::<u32>(ERROR_OFFSET, 7);
        return -7;
    }
    // The barrier connection only proves that registration completed. Normal
    // clients obtain their own attenuated connection through a lookup.
    ipc_close(replacement_connection);
    config::write::<u32>(STAGE_OFFSET, 5);
    replacement_asid as i64
}

catten_rt::entry!(main);
