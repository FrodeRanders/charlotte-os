#![allow(unused_unsafe)]
//! Net client — sends a test Ethernet frame through the virtio-net driver.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{Args, Input, config};
use catten_services::{net, ns, wait_reply};
use catten_syscall::{
    ipc_scalar_call, ipc_scalar_call_move,
    memory_alloc, memory_map, memory_unmap,
    thread_exit,
};

const REPLY_SPINS: u64 = 50_000_000;
const LOOKUP_ATTEMPTS: u64 = 1_000_000;
const SENTINEL: u32 = 0xC0DE;
const STAGE_OFFSET: usize = 12;
const TX_SUCCESS_OFFSET: usize = 4;

/// A minimal Ethernet frame (broadcast, EtherType 0x0800 = IPv4, payload
/// all zeros).  It's ~64 bytes so SLIRP won't drop the short frame.
const TEST_FRAME: [u8; 64] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // dst MAC = broadcast
    0x52, 0x54, 0x00, 0x12, 0x34, 0x56, // src MAC = fake QEMU
    0x08, 0x00,                         // EtherType = IPv4
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,   // padding (46 bytes)
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,
];

fn cmain(_args: Args, _input: Input<0>) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1);
    let ns_conn = match config::bootstrap_cap() { Some(c) => c, None => unsafe { thread_exit() } };
    config::write::<u32>(STAGE_OFFSET, 2);

    let mut attempts: u64 = 0;
    let net_conn = loop {
        let l = unsafe { ipc_scalar_call(ns_conn, ns::OP_LOOKUP, net::NAME) };
        if l != 0 {
            let (r, cap) = unsafe { wait_reply(l, REPLY_SPINS) };
            if r >= 1 && cap != 0 { break cap; }
        }
        attempts += 1;
        if attempts >= LOOKUP_ATTEMPTS { unsafe { thread_exit() }; }
        core::hint::spin_loop();
    };
    config::write::<u32>(STAGE_OFFSET, 3);

    // Allocate a page for the frame, write the test payload.
    let frame_cap = unsafe { memory_alloc(1) };
    if frame_cap == 0 { unsafe { thread_exit() }; }
    let frame_vaddr: usize = 0x0000_0000_0060_0000;
    if unsafe { memory_map(frame_cap, frame_vaddr, true) } != 0 {
        unsafe { thread_exit() };
    }
    unsafe {
        core::ptr::copy_nonoverlapping(TEST_FRAME.as_ptr(), frame_vaddr as *mut u8, TEST_FRAME.len());
        memory_unmap(frame_cap);
    }

    // Send through OP_SEND (call with moved memory object).
    let call = unsafe { ipc_scalar_call_move(net_conn, net::OP_SEND, TEST_FRAME.len() as u64, frame_cap) };
    if call == 0 { unsafe { thread_exit() }; }
    let (status, _) = unsafe { wait_reply(call, REPLY_SPINS) };
    config::write::<u32>(TX_SUCCESS_OFFSET, status as u32);
    config::write::<u32>(STAGE_OFFSET, 4); // TX attempted

    config::write::<u32>(0, SENTINEL);
    unsafe { thread_exit() };
}

catten_rt::entry!(cmain);
