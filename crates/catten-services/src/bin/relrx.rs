//! Blocking NIC receive pump for the reliable-message service.
#![no_std]
#![no_main]

use catten_rt::{Context, config};
use catten_services::{
    net,
    ns,
    relmsg,
    wait_reply,
};
use catten_syscall::{
    ipc_close,
    ipc_reply_wait_with_memory,
    ipc_scalar_call,
    ipc_scalar_call_move,
    memory_close,
    thread_exit,
};

fn lookup(ns_conn: u64, name: u64) -> u64 {
    let call = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, name);
    if call == 0 {
        return 0;
    }
    let (generation, connection) = unsafe { wait_reply(call, 0) };
    if generation < 1 {
        0
    } else {
        connection
    }
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(0, 1);
    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let net_conn = lookup(ns_conn, net::NAME);
    let relmsg_conn = lookup(ns_conn, relmsg::NAME);
    if net_conn == 0 || relmsg_conn == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 2);

    loop {
        config::write::<u32>(0, 3);
        let receive = ipc_scalar_call(net_conn, net::OP_RECV, 0);
        if receive == 0 {
            unsafe { thread_exit() };
        }
        config::write::<u32>(0, 4);
        let (status, frame_len, _connection, memory) = ipc_reply_wait_with_memory(receive);
        config::write::<u32>(0, 5);
        ipc_close(receive);
        if status != 0 || memory == 0 || frame_len > 4096 {
            if memory != 0 {
                memory_close(memory);
            }
            continue;
        }
        let forward = ipc_scalar_call_move(relmsg_conn, relmsg::OP_FRAME, frame_len, memory);
        if forward == 0 {
            memory_close(memory);
            continue;
        }
        config::write::<u32>(0, 6);
        let _ = unsafe { wait_reply(forward, 0) };
        let forwarded = unsafe { config::read::<u32>(4) };
        config::write::<u32>(4, forwarded.wrapping_add(1));
    }
}

catten_rt::entry!(main);
