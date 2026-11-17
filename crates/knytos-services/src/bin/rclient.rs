//! Two-node reliable-message smoke client.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Context,
    config,
    owned::OwnedMemory,
};
use catten_services::{
    relmsg,
    wait_for_registered_name_owned,
};
use catten_syscall::thread_exit;
use charlotte_launch::relmsg_client_status as status;
use charlotte_protocol_msg::{
    IPC_MESSAGE_HEADER_SIZE,
    build_ipc_message_header,
    parse_ipc_message_header,
    unpack_mac,
};

const SENTINEL: u32 = 0xc0de_cafe;
/// Larger than v2's 65,535-byte ceiling, exercising v3's 32-bit IPC and wire
/// lengths as well as fragmentation/reassembly in both directions.
const PAYLOAD_LEN: usize = 70_000;

fn build_payload() -> alloc::vec::Vec<u8> {
    let mut payload = alloc::vec::Vec::with_capacity(PAYLOAD_LEN);
    for i in 0..PAYLOAD_LEN {
        payload.push((i as u8).wrapping_mul(31).wrapping_add(7));
    }
    payload
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let payload = build_payload();
    let ns_conn = match ctx.bootstrap_connection() {
        Some(connection) => connection,
        None => unsafe { thread_exit() },
    };
    let (_, relmsg_conn) = wait_for_registered_name_owned(ns_conn, relmsg::NAME)
        .unwrap_or_else(|| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 2);

    let status_call = relmsg_conn.call(relmsg::OP_STATUS, 0).unwrap_or_else(|_| {
        config::write::<u32>(status::STAGE, 0xe001);
        unsafe { thread_exit() }
    });
    config::write::<u32>(status::STAGE, 10);
    let packed = status_call.wait().unwrap_or_else(|_| unsafe { thread_exit() }).result;
    config::write::<u32>(status::STAGE, 11);
    let local_mac = unpack_mac(packed as u64);
    let mut peer_mac = local_mac;
    peer_mac[5] = match local_mac[5] {
        1 => 2,
        2 => 1,
        _ => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 12);

    // This verifier is itself part of the network-ready condition, so waiting
    // for the kernel's boot-done marker here would be circular. The kernel
    // launches it only after frouter is serving; relmsg retransmission covers
    // bounded peer-start skew.
    config::write::<u32>(status::STAGE, 13);

    let receive = relmsg_conn.call(relmsg::OP_RECV, 0).unwrap_or_else(|_| {
        config::write::<u32>(status::STAGE, 0xe002);
        unsafe { thread_exit() }
    });
    config::write::<u32>(status::STAGE, 13);
    let object_len = IPC_MESSAGE_HEADER_SIZE + payload.len();
    let memory = OwnedMemory::allocate(object_len.div_ceil(4096))
        .unwrap_or_else(|_| unsafe { thread_exit() });
    let mut mapping = memory.map_writable().unwrap_or_else(|_| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 14);
    let mut header = [0u8; IPC_MESSAGE_HEADER_SIZE];
    build_ipc_message_header(&mut header, peer_mac, payload.len() as u32);
    mapping.as_mut_slice()[..IPC_MESSAGE_HEADER_SIZE].copy_from_slice(&header);
    mapping.as_mut_slice()[IPC_MESSAGE_HEADER_SIZE..object_len].copy_from_slice(&payload);
    let memory = mapping.unmap().unwrap_or_else(|_| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 15);
    let send = relmsg_conn.call_move(relmsg::OP_SEND, 0, memory).unwrap_or_else(|_| {
        config::write::<u32>(status::STAGE, 0xe003);
        unsafe { thread_exit() }
    });
    config::write::<u32>(status::STAGE, 16);
    let send_result = send.wait().unwrap_or_else(|_| unsafe { thread_exit() }).result;
    if send_result != payload.len() as i64 {
        config::write::<i64>(status::SEND_RESULT, send_result);
        config::write::<u32>(status::STAGE, 0xe004);
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 3);

    let received = receive.wait().unwrap_or_else(|_| unsafe { thread_exit() });
    if received.result != payload.len() as i64 {
        unsafe { thread_exit() };
    }
    let memory = received.memory.unwrap_or_else(|| unsafe { thread_exit() });
    let mapping = memory.map_read_only().unwrap_or_else(|_| unsafe { thread_exit() });
    let envelope =
        parse_ipc_message_header(mapping.as_slice()).unwrap_or_else(|_| unsafe { thread_exit() });
    if envelope.peer != peer_mac || envelope.payload_len as usize != payload.len() {
        unsafe { thread_exit() };
    }
    let matches = &mapping.as_slice()
        [IPC_MESSAGE_HEADER_SIZE..IPC_MESSAGE_HEADER_SIZE + envelope.payload_len as usize]
        == payload.as_slice();
    if !matches {
        unsafe { thread_exit() };
    }

    if let Ok(shutdown) = relmsg_conn.call(relmsg::OP_SHUTDOWN, 0) {
        let _ = shutdown.wait();
    }
    config::write::<u32>(
        status::PEER_ADDRESS,
        u32::from_be_bytes([peer_mac[2], peer_mac[3], peer_mac[4], peer_mac[5]]),
    );
    config::write::<u32>(status::STAGE, SENTINEL);
    unsafe { thread_exit() };
}

catten_rt::entry!(main);
