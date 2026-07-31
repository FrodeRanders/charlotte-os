//! Two-node reliable-message smoke client.
#![no_std]
#![no_main]

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    ns,
    relmsg,
    wait_reply,
};
use catten_syscall::{
    ipc_close,
    ipc_reply_wait_with_memory,
    ipc_scalar_call,
    ipc_scalar_call_move,
    memory_alloc,
    memory_close,
    memory_map,
    memory_unmap,
    thread_exit,
};
use charlotte_protocol_msg::{
    pack_address_and_len,
    unpack_address_and_len,
};

const SCRATCH: usize = 0x0000_0000_00a0_0000;
const SENTINEL: u32 = 0xc0de_cafe;
const PAYLOAD: &[u8] = b"CharlotteOS relmsg cross-node";

fn main(ctx: Context) -> ! {
    config::write::<u32>(0, 1);
    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, relmsg::NAME);
    if lookup == 0 {
        unsafe { thread_exit() };
    }
    let (generation, relmsg_conn) = unsafe { wait_reply(lookup, 0) };
    if generation < 1 || relmsg_conn == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 2);

    let status = ipc_scalar_call(relmsg_conn, relmsg::OP_STATUS, 0);
    if status == 0 {
        config::write::<u32>(0, 0xe001);
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 10);
    let (packed, _) = unsafe { wait_reply(status, 0) };
    config::write::<u32>(0, 11);
    let (local_mac, _) = unpack_address_and_len(packed as u64);
    let mut peer_mac = local_mac;
    peer_mac[5] = match local_mac[5] {
        1 => 2,
        2 => 1,
        _ => unsafe { thread_exit() },
    };
    config::write::<u32>(0, 12);

    // Do not initiate cluster communication until this node has finished
    // booting: messages sent during the boot storm are silently lost.
    if !catten_services::wait_for_boot_done(ns_conn) {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 13);

    let receive = ipc_scalar_call(relmsg_conn, relmsg::OP_RECV, 0);
    if receive == 0 {
        config::write::<u32>(0, 0xe002);
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 13);
    let cap = memory_alloc(1);
    if cap == 0 || memory_map(cap, SCRATCH, true) != 0 {
        if cap != 0 {
            memory_close(cap);
        }
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 14);
    unsafe {
        core::ptr::copy_nonoverlapping(PAYLOAD.as_ptr(), SCRATCH as *mut u8, PAYLOAD.len());
    }
    memory_unmap(cap);
    config::write::<u32>(0, 15);
    let destination = pack_address_and_len(peer_mac, PAYLOAD.len() as u16);
    let send = ipc_scalar_call_move(relmsg_conn, relmsg::OP_SEND, destination, cap);
    if send == 0 {
        config::write::<u32>(0, 0xe003);
        memory_close(cap);
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 16);
    let (send_result, _) = unsafe { wait_reply(send, 0) };
    if send_result != PAYLOAD.len() as i64 {
        config::write::<i64>(8, send_result);
        config::write::<u32>(0, 0xe004);
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 3);

    let (status, source_and_len, _connection, received) = ipc_reply_wait_with_memory(receive);
    ipc_close(receive);
    let (source, len) = unpack_address_and_len(source_and_len);
    if status != 0 || received == 0 || source != peer_mac || len as usize != PAYLOAD.len() {
        if received != 0 {
            memory_close(received);
        }
        unsafe { thread_exit() };
    }
    if memory_map(received, SCRATCH, false) != 0 {
        memory_close(received);
        unsafe { thread_exit() };
    }
    let bytes = unsafe { core::slice::from_raw_parts(SCRATCH as *const u8, len as usize) };
    let matches = bytes == PAYLOAD;
    memory_unmap(received);
    memory_close(received);
    if !matches {
        unsafe { thread_exit() };
    }

    let shutdown = ipc_scalar_call(relmsg_conn, relmsg::OP_SHUTDOWN, 0);
    if shutdown != 0 {
        let _ = unsafe { wait_reply(shutdown, 0) };
    }
    config::write::<u32>(
        4,
        u32::from_be_bytes([peer_mac[2], peer_mac[3], peer_mac[4], peer_mac[5]]),
    );
    config::write::<u32>(0, SENTINEL);
    unsafe { thread_exit() };
}

catten_rt::entry!(main);
