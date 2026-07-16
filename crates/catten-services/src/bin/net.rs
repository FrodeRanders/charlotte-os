//! The reference userspace virtio-net driver (Phase 9).
//!
//! Runs in an isolated EL0 protection domain, owning the virtio-net PCI
//! device through *delegated* device capabilities only — an MMIO region
//! (BAR0) and an interrupt. It never names a physical address or interrupt
//! vector. It runs the virtio init sequence, reads the MAC, registers a
//! network endpoint by name, and serves status queries.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Args,
    Input,
    config,
};
use catten_services::{
    net,
    ns,
    virtio,
    wait_reply,
};
use catten_syscall::{
    device_irq_bind_cq,
    device_mmio_map,
    device_mmio_unmap,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_scalar_call_connection,
    ipc_status,
    thread_exit,
    IpcRights,
    cq_wait,
};

const REPLY_SPINS: u64 = 50_000_000;
const UART_MMIO_VADDR: usize = 0x0000_0000_0040_0000;
const STAGE_OFFSET: usize = 0;
const MAC_OFFSET: usize = 4; // 6 bytes of MAC at offsets 4..10
const MAC_PRESENT: usize = 12; // u32: 1 when MAC has been read

#[inline]
unsafe fn read8(addr: usize) -> u8 {
    unsafe { core::ptr::read_volatile(addr as *const u8) }
}

#[inline]
unsafe fn read32(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline]
unsafe fn write8(addr: usize, value: u8) {
    unsafe { core::ptr::write_volatile(addr as *mut u8, value) }
}

#[inline]
unsafe fn write16(addr: usize, value: u16) {
    unsafe { core::ptr::write_volatile(addr as *mut u16, value) }
}

#[inline]
unsafe fn write32(addr: usize, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

fn cmain(_args: Args, _input: Input<0>) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1); // started

    let ns_connection = match config::bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let mmio_cap = match config::mmio_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let irq_cap = match config::irq_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(STAGE_OFFSET, 2); // grants received

    if unsafe { device_mmio_map(mmio_cap, UART_MMIO_VADDR, true) } != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 3); // BAR0 mapped

    let bar0 = UART_MMIO_VADDR + virtio::COMMON_CFG_OFFSET;

    // --- virtio init sequence (legacy transport) ---
    // 1. Reset
    unsafe { write8(bar0 + virtio::DEVICE_STATUS, 0) };
    // 2. ACKNOWLEDGE
    unsafe { write8(bar0 + virtio::DEVICE_STATUS, virtio::STATUS_ACKNOWLEDGE) };
    // 3. DRIVER
    unsafe {
        write8(bar0 + virtio::DEVICE_STATUS,
               virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER)
    };
    // 4. Negotiate features: accept none for now (simplest path)
    let _features = unsafe { read32(bar0 + virtio::DEVICE_FEATURES) };
    // 5. FEATURES_OK
    unsafe {
        write8(bar0 + virtio::DEVICE_STATUS,
               virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER
               | virtio::STATUS_FEATURES_OK)
    };
    let status = unsafe { read8(bar0 + virtio::DEVICE_STATUS) };
    if status & virtio::STATUS_FEATURES_OK == 0 {
        // Features not accepted; try without FEATURES_OK.
        unsafe {
            write8(bar0 + virtio::DEVICE_STATUS,
                   virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER)
        };
    }
    // 6. Read device-specific config (MAC, link status)
    let dev_cfg = UART_MMIO_VADDR + virtio::COMMON_CFG_OFFSET + virtio::DEVICE_CFG_OFFSET;
    for i in 0..6u8 {
        config::write::<u8>(MAC_OFFSET + i as usize, unsafe { read8(dev_cfg + virtio::NET_MAC + i as usize) });
    }
    config::write::<u32>(MAC_PRESENT, 1);

    // 7. DRIVER_OK — device is live
    unsafe {
        write8(bar0 + virtio::DEVICE_STATUS,
               virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER
               | virtio::STATUS_DRIVER_OK
               | virtio::STATUS_FEATURES_OK)
    };
    config::write::<u32>(STAGE_OFFSET, 4); // driver OK

    // Register the endpoint by name.
    let endpoint = unsafe { ipc_endpoint_create(net::INTERFACE, net::VERSION, 8) };
    if endpoint == 0 {
        unsafe { thread_exit() };
    }
    let register = unsafe {
        ipc_scalar_call_connection(
            ns_connection,
            ns::OP_REGISTER,
            net::NAME,
            endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
    };
    if register == 0 {
        unsafe { thread_exit() };
    }
    let (generation, _) = unsafe { wait_reply(register, REPLY_SPINS) };
    if generation < 1 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 5); // registered

    // Serve status queries.
    if unsafe { ipc_endpoint_bind_cq(endpoint, 0) } != 0 {
        unsafe { thread_exit() };
    }
    if unsafe { device_irq_bind_cq(irq_cap, 0) } != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 6); // serving

    loop {
        unsafe { cq_wait(1, 0) };
        loop {
            let message = unsafe { ipc_recv(endpoint) };
            if message.status == ipc_status::NO_MESSAGE {
                break;
            }
            if message.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() };
            }
            if !message.is_ok() {
                break;
            }
            match message.opcode {
                net::OP_STATUS => {
                    if message.reply != 0 {
                        // Pack MAC bytes 0..5 into reply bits 8..55, link
                        // status into bits 0..7. Read from config so the
                        // verifier can check the MAC via config page.
                        let dev_cfg = UART_MMIO_VADDR + virtio::COMMON_CFG_OFFSET + virtio::DEVICE_CFG_OFFSET;
                        let mac0 = unsafe { read8(dev_cfg + virtio::NET_MAC + 0) } as u64;
                        let mac1 = unsafe { read8(dev_cfg + virtio::NET_MAC + 1) } as u64;
                        let mac2 = unsafe { read8(dev_cfg + virtio::NET_MAC + 2) } as u64;
                        let mac3 = unsafe { read8(dev_cfg + virtio::NET_MAC + 3) } as u64;
                        let mac4 = unsafe { read8(dev_cfg + virtio::NET_MAC + 4) } as u64;
                        let mac5 = unsafe { read8(dev_cfg + virtio::NET_MAC + 5) } as u64;
                        let link = unsafe { read32(dev_cfg + virtio::NET_STATUS) } as u64;
                        let result = (link & 0xff) | (mac0 << 8) | (mac1 << 16)
                            | (mac2 << 24) | (mac3 << 32) | (mac4 << 40) | (mac5 << 48);
                        unsafe { ipc_reply(message.reply, result as i64) };
                    }
                }
                net::OP_SHUTDOWN => {
                    if message.reply != 0 {
                        unsafe { ipc_reply(message.reply, 0) };
                    }
                    unsafe {
                        device_mmio_unmap(mmio_cap);
                        catten_syscall::device_close(irq_cap);
                        thread_exit();
                    }
                }
                _ => {
                    if message.reply != 0 {
                        unsafe { ipc_reply(message.reply, -1) };
                    }
                }
            }
        }
    }
}

catten_rt::entry!(cmain);
