//! The CharlotteOS service manager — orchestrates live service upgrades.
//!
//! Bootstraps, registers as "svcmgr", and serves OP_UPGRADE requests.
//! An upgrade drains the target service, receives its handoff state
//! (memory objects + endpoint cap), records them in the config page, and
//! notifies the kernel supervisor to spawn the replacement.
//!
//! ## Current prototype scope
//!
//! The orchestration through OP_HANDOFF is implemented.  The domain-spawn
//! step is not yet exposed to EL0 (there is no SPAWN_DOMAIN syscall); the
//! manager writes the handoff state to its config page and the kernel
//! self-test verifier picks it up.  A future `SPAWN_DOMAIN` syscall or a
//! supervisor endpoint would complete the EL0-only path.
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
    ipc_reply_poll,
    ipc_reply_poll_with_memory,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_status,
    memory_close,
    thread_exit,
};

const REPLY_SPINS: u64 = 50_000_000;
const LOOKUP_ATTEMPTS: u64 = 2_000_000;

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

/// Spin until a pending call completes.  Returns `(result, returned_cap)`.
unsafe fn spin_call(call: u64, what: &str) -> (u64, u64) {
    let mut spins: u64 = 0;
    loop {
        let (status, result, cap) = ipc_reply_poll(call);
        if status == 0 {
            ipc_close(call);
            return (result, cap);
        }
        spins += 1;
        if spins >= REPLY_SPINS {
            unsafe { thread_exit() };
        }
        core::hint::spin_loop();
    }
    let _ = what;
}

/// Look up a service by short name; return its connection cap.
unsafe fn lookup(ns_conn: u64, target: u64) -> Option<u64> {
    let mut attempts = 0u64;
    loop {
        let l = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, target);
        if l != 0 {
            let (generation, cap) = unsafe { spin_call(l, "lookup") };
            if generation >= 1 && cap != 0 {
                return Some(cap);
            }
        }
        attempts += 1;
        if attempts >= LOOKUP_ATTEMPTS {
            return None;
        }
        core::hint::spin_loop();
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
    let target_conn = match unsafe { lookup(ns_conn, target_name) } {
        Some(c) => c,
        None => {
            config::write::<u32>(ERROR_OFFSET, 1);
            return -1;
        }
    };

    // OP_HANDOFF: the target serialises state, returns (state_cap, ep_cap),
    // and exits.  We use ipc_reply_poll_with_memory to capture the moved
    // memory cap.
    let call = ipc_scalar_call(target_conn, echo::OP_HANDOFF, 0);
    if call == 0 {
        config::write::<u32>(ERROR_OFFSET, 2);
        return -2;
    }

    let mut state_cap: u64 = 0;
    let mut handoff_result: u64 = 0;
    {
        let mut spins: u64 = 0;
        loop {
            let (status, result, _conn, mem) = ipc_reply_poll_with_memory(call);
            if status == 0 {
                state_cap = mem;
                handoff_result = result;
                break;
            }
            spins += 1;
            if spins >= REPLY_SPINS {
                config::write::<u32>(ERROR_OFFSET, 3);
                return -3;
            }
            core::hint::spin_loop();
        }
    }
    catten_syscall::ipc_close(call);

    if state_cap == 0 {
        config::write::<u32>(ERROR_OFFSET, 4);
        return -4;
    }

    // Decode the endpoint cap from the handoff result.
    let ep_cap = handoff_result >> 16;

    // Record the handoff state for the kernel supervisor/verifier.
    config::write::<u64>(STATE_CAP_OFFSET, state_cap);
    config::write::<u64>(ENDPOINT_CAP_OFFSET, ep_cap);
    config::write::<u32>(STAGE_OFFSET, 4); // handoff complete, awaiting replacement

    // The state cap is now in our address space.  The kernel verifier
    // reads the cap ID from our config page and moves it to the
    // replacement domain.  Do NOT close it here — the verifier needs it.
    config::write::<u32>(STAGE_OFFSET, 5); // state delivered

    0 // success
}

catten_rt::entry!(main);
