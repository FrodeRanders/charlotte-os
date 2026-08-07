//! Two-node reliable-message smoke client.
#![no_std]
#![no_main]

extern crate alloc;

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
    memory_map_any,
    memory_unmap,
    thread_exit,
};
use charlotte_protocol_msg::{
    pack_address_and_len,
    unpack_address_and_len,
};

const SENTINEL: u32 = 0xc0de_cafe;
/// Large enough to span multiple relmsg frames (~1468 each), exercising the
/// fragmentation/reassembly path on both the send and receive sides.
const PAYLOAD_LEN: usize = 3000;

fn build_payload() -> alloc::vec::Vec<u8> {
    let mut payload = alloc::vec::Vec::with_capacity(PAYLOAD_LEN);
    for i in 0..PAYLOAD_LEN {
        payload.push((i as u8).wrapping_mul(31).wrapping_add(7));
    }
    payload
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(0, 1);
    let payload = build_payload();
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
    if !catten_services::wait_for_local_ready(ns_conn) {
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
        let (scratch_2_map_status, scratch_2_vaddr) = memory_map_any(cap, true);
    if cap == 0 || scratch_2_map_status != 0 {
        if cap != 0 {
            memory_close(cap);
        }
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 14);
    unsafe {
        core::ptr::copy_nonoverlapping(payload.as_ptr(), scratch_2_vaddr as *mut u8, payload.len());
    }
    memory_unmap(cap);
    config::write::<u32>(0, 15);
    let destination = pack_address_and_len(peer_mac, payload.len() as u16);
    let send = ipc_scalar_call_move(relmsg_conn, relmsg::OP_SEND, destination, cap);
    if send == 0 {
        config::write::<u32>(0, 0xe003);
        memory_close(cap);
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 16);
    let (send_result, _) = unsafe { wait_reply(send, 0) };
    if send_result != payload.len() as i64 {
        config::write::<i64>(8, send_result);
        config::write::<u32>(0, 0xe004);
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 3);

    let (status, source_and_len, _connection, received) = ipc_reply_wait_with_memory(receive);
    ipc_close(receive);
    let (source, len) = unpack_address_and_len(source_and_len);
    if status != 0 || received == 0 || source != peer_mac || len as usize != payload.len() {
        if received != 0 {
            memory_close(received);
        }
        unsafe { thread_exit() };
    }
        let (scratch_map_status, scratch_vaddr) = memory_map_any(received, false);
    if scratch_map_status != 0 {
        memory_close(received);
        unsafe { thread_exit() };
    }
    let bytes = unsafe { core::slice::from_raw_parts(scratch_vaddr as *const u8, len as usize) };
    let matches = bytes == payload.as_slice();
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
