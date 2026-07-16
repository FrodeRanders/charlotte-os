//! The reference userspace virtio-net driver (Phase 9).
//!
//! Maps BAR0, runs the virtio init sequence, reads the MAC from device
//! config, allocates a page and demonstrates the new `memory_get_phys`
//! syscall (reports the physical address to the verifier), registers a
//! "net0" endpoint, and serves OP_STATUS.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{Args, Input, config};
use catten_services::{net, ns, virtio, wait_reply};
use catten_syscall::{
    device_irq_ack, device_irq_bind_cq, device_mmio_map, device_mmio_unmap,
    ipc_endpoint_bind_cq, ipc_endpoint_create, ipc_recv, ipc_reply,
    ipc_scalar_call_connection,
    memory_alloc, memory_get_phys,
    thread_exit, IpcRights, cq_wait, ipc_status,
};

const REPLY_SPINS: u64 = 50_000_000;
const VADDR_BAR0: usize = 0x0000_0000_0040_0000;
const STAGE_OFFSET: usize = 0;

#[inline] unsafe fn r8(a: usize) -> u8  { unsafe { core::ptr::read_volatile(a as *const u8) } }
#[inline] unsafe fn r16(a: usize) -> u16 { unsafe { core::ptr::read_volatile(a as *const u16) } }
#[inline] unsafe fn r32(a: usize) -> u32 { unsafe { core::ptr::read_volatile(a as *const u32) } }
#[inline] unsafe fn w8(a: usize, v: u8)   { unsafe { core::ptr::write_volatile(a as *mut u8, v) } }
#[inline] unsafe fn w16(a: usize, v: u16) { unsafe { core::ptr::write_volatile(a as *mut u16, v) } }
#[inline] unsafe fn w32(a: usize, v: u32) { unsafe { core::ptr::write_volatile(a as *mut u32, v) } }

fn cmain(_args: Args, _input: Input<0>) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1);
    let ns_connection = match config::bootstrap_cap() {
        Some(c) => c, None => unsafe { thread_exit() },
    };
    let mmio_cap = match config::mmio_cap() { Some(c) => c, None => unsafe { thread_exit() } };
    let irq_cap = match config::irq_cap() { Some(c) => c, None => unsafe { thread_exit() } };
    config::write::<u32>(STAGE_OFFSET, 2);

    if unsafe { device_mmio_map(mmio_cap, VADDR_BAR0, true) } != 0 { unsafe { thread_exit() }; }
    config::write::<u32>(STAGE_OFFSET, 3);
    let bar0 = VADDR_BAR0 + virtio::COMMON_CFG_OFFSET;

    // --- virtio init ---
    unsafe { w8(bar0 + virtio::DEVICE_STATUS, 0) };
    unsafe { w8(bar0 + virtio::DEVICE_STATUS, virtio::STATUS_ACKNOWLEDGE) };
    unsafe { w8(bar0 + virtio::DEVICE_STATUS, virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER) };
    let _ = unsafe { r32(bar0 + virtio::DEVICE_FEATURES) };
    unsafe {
        w8(bar0 + virtio::DEVICE_STATUS,
           virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER | virtio::STATUS_FEATURES_OK)
    };
    let st = unsafe { r8(bar0 + virtio::DEVICE_STATUS) };
    if st & virtio::STATUS_FEATURES_OK == 0 {
        unsafe { w8(bar0 + virtio::DEVICE_STATUS,
                    virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER) };
    }

    // Read MAC.
    let dc = VADDR_BAR0 + virtio::COMMON_CFG_OFFSET + virtio::DEVICE_CFG_OFFSET;
    for i in 0..6 { config::write::<u8>(4 + i, unsafe { r8(dc + virtio::NET_MAC + i) }); }
    config::write::<u32>(STAGE_OFFSET, 4);

    // Allocate a page via memory_get_phys — proves the new syscall works from EL0.
    let test_cap = unsafe { memory_alloc(1) };
    let test_phys = unsafe { memory_get_phys(test_cap) };
    if test_phys == 0 { unsafe { thread_exit() }; }
    config::write::<u64>(16, test_phys);
    config::write::<u32>(STAGE_OFFSET, 5);

    // DRIVER_OK
    unsafe {
        w8(bar0 + virtio::DEVICE_STATUS,
           virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER
           | virtio::STATUS_DRIVER_OK | virtio::STATUS_FEATURES_OK)
    };
    config::write::<u32>(STAGE_OFFSET, 6);

    // Register endpoint.
    let endpoint = unsafe { ipc_endpoint_create(net::INTERFACE, net::VERSION, 8) };
    if endpoint == 0 { unsafe { thread_exit() }; }
    let reg = unsafe {
        ipc_scalar_call_connection(ns_connection, ns::OP_REGISTER, net::NAME, endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION)
    };
    if reg == 0 { unsafe { thread_exit() }; }
    let (generation, _) = unsafe { wait_reply(reg, REPLY_SPINS) };
    if generation < 1 { unsafe { thread_exit() }; }
    config::write::<u32>(STAGE_OFFSET, 7);

    // Bind and serve.
    if unsafe { ipc_endpoint_bind_cq(endpoint, 0) } != 0 { unsafe { thread_exit() }; }
    if unsafe { device_irq_bind_cq(irq_cap, 0) } != 0 { unsafe { thread_exit() }; }
    config::write::<u32>(STAGE_OFFSET, 8);

    loop {
        unsafe { cq_wait(1, 0) };
        let (_s, _c) = unsafe { device_irq_ack(irq_cap) };

        loop {
            let m = unsafe { ipc_recv(endpoint) };
            if m.status == ipc_status::NO_MESSAGE { break; }
            if m.status == ipc_status::ENDPOINT_CLOSED { unsafe { thread_exit() }; }
            if !m.is_ok() { break; }
            match m.opcode {
                net::OP_STATUS => {
                    if m.reply != 0 {
                        let d = VADDR_BAR0 + virtio::COMMON_CFG_OFFSET + virtio::DEVICE_CFG_OFFSET;
                        let mac = [
                            unsafe { r8(d + virtio::NET_MAC + 0) } as u64,
                            unsafe { r8(d + virtio::NET_MAC + 1) } as u64,
                            unsafe { r8(d + virtio::NET_MAC + 2) } as u64,
                            unsafe { r8(d + virtio::NET_MAC + 3) } as u64,
                            unsafe { r8(d + virtio::NET_MAC + 4) } as u64,
                            unsafe { r8(d + virtio::NET_MAC + 5) } as u64,
                        ];
                        let link = unsafe { r16(d + virtio::NET_STATUS) } as u64;
                        let result = (link & 1) | (mac[0] << 8) | (mac[1] << 16)
                            | (mac[2] << 24) | (mac[3] << 32) | (mac[4] << 40) | (mac[5] << 48);
                        unsafe { ipc_reply(m.reply, result as i64) };
                    }
                }
                net::OP_SHUTDOWN => {
                    if m.reply != 0 { unsafe { ipc_reply(m.reply, 0) }; }
                    unsafe { device_mmio_unmap(mmio_cap); catten_syscall::device_close(irq_cap); thread_exit(); }
                }
                _ => { if m.reply != 0 { unsafe { ipc_reply(m.reply, -1) }; } }
            }
        }
    }
}

catten_rt::entry!(cmain);
