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
    wait_for_local_ready_owned,
    wait_for_registered_name_owned,
};
use catten_syscall::thread_exit;
use charlotte_launch::relmsg_client_status as status;
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
    let (local_mac, _) = unpack_address_and_len(packed as u64);
    let mut peer_mac = local_mac;
    peer_mac[5] = match local_mac[5] {
        1 => 2,
        2 => 1,
        _ => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 12);

    // Do not initiate cluster communication until this node has finished
    // booting: messages sent during the boot storm are silently lost.
    if !wait_for_local_ready_owned(ns_conn) {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 13);

    let receive = relmsg_conn.call(relmsg::OP_RECV, 0).unwrap_or_else(|_| {
        config::write::<u32>(status::STAGE, 0xe002);
        unsafe { thread_exit() }
    });
    config::write::<u32>(status::STAGE, 13);
    let memory = OwnedMemory::allocate(1).unwrap_or_else(|_| unsafe { thread_exit() });
    let mut mapping = memory.map_writable().unwrap_or_else(|_| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 14);
    mapping.as_mut_slice()[..payload.len()].copy_from_slice(&payload);
    let memory = mapping.unmap().unwrap_or_else(|_| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 15);
    let destination = pack_address_and_len(peer_mac, payload.len() as u16);
    let send = relmsg_conn.call_move(relmsg::OP_SEND, destination, memory).unwrap_or_else(|_| {
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
    let (source, len) = unpack_address_and_len(received.result as u64);
    if source != peer_mac || len as usize != payload.len() {
        unsafe { thread_exit() };
    }
    let memory = received.memory.unwrap_or_else(|| unsafe { thread_exit() });
    let mapping = memory.map_read_only().unwrap_or_else(|_| unsafe { thread_exit() });
    let matches = &mapping.as_slice()[..len as usize] == payload.as_slice();
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
