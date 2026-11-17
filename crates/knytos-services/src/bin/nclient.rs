//! Net client — sends a test Ethernet frame through the virtio-net driver.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Context,
    config,
    owned::OwnedMemory,
};
use catten_services::{
    net,
    wait_for_registered_name_owned,
};
use catten_syscall::thread_exit;
use charlotte_launch::net_client_status as status;

const SENTINEL: u32 = 0xc0de;

/// A minimal Ethernet frame (broadcast, EtherType 0x0800 = IPv4, payload
/// all zeros).  It's ~64 bytes so SLIRP won't drop the short frame.
const TEST_FRAME: [u8; 64] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // dst MAC = broadcast
    0x52, 0x54, 0x00, 0x12, 0x34, 0x56, // src MAC = fake QEMU
    0x08, 0x00, // EtherType = IPv4
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // padding (46 bytes)
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0,
];

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let ns_conn = match ctx.bootstrap_connection() {
        Some(connection) => connection,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 2);

    let (_, net_conn) = wait_for_registered_name_owned(ns_conn, net::NAME)
        .unwrap_or_else(|| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 3);

    // Allocate a page for the frame, write the test payload.
    let frame = OwnedMemory::allocate(1).unwrap_or_else(|_| unsafe { thread_exit() });
    let mut mapping = frame.map_writable().unwrap_or_else(|_| unsafe { thread_exit() });
    mapping.as_mut_slice()[..TEST_FRAME.len()].copy_from_slice(&TEST_FRAME);
    let frame = mapping.unmap().unwrap_or_else(|_| unsafe { thread_exit() });

    // Transfer the frame object to the NIC without copying its payload.
    let call = net_conn
        .call_move(net::OP_SEND, TEST_FRAME.len() as u64, frame)
        .unwrap_or_else(|_| unsafe { thread_exit() });
    let result = call.wait().unwrap_or_else(|_| unsafe { thread_exit() }).result;
    config::write::<u32>(status::TX_RESULT, result as u32);
    config::write::<u32>(status::STAGE, 4); // TX attempted

    // The NIC driver is deliberately left running: cluster discovery, the
    // frame demultiplexer, and the reliable-message layer all keep using it
    // after this smoke client finishes. Tearing it down here would strand
    // their in-flight OP_SEND and OP_RECV calls.
    config::write::<u32>(status::SENTINEL, SENTINEL);
    unsafe { thread_exit() };
}

catten_rt::entry!(main);
