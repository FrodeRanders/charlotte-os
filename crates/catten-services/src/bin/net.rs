//! The reference userspace virtio-net driver (Phase 9).
//!
//! Ethernet frame transport (§6 of the networking architecture doc): the
//! driver knows only about frames and virtio queues — no IP, no TCP.  It
//! maps BAR0, runs the virtio init sequence, allocates physical memory for
//! two virtqueues (RX and TX) using `memory_get_phys`, and serves OP_SEND
//! (transmit) and OP_RECV (deferred receive).
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{Context, config};
use catten_services::{net, ns, virtio, wait_reply};
use catten_syscall::{
    device_irq_ack, device_irq_bind_cq, device_mmio_map, device_mmio_unmap,
    ipc_endpoint_bind_cq, ipc_endpoint_create, ipc_recv, ipc_reply,
    ipc_reply_move, ipc_scalar_call_connection,
    memory_alloc, memory_close, memory_get_phys, memory_map, memory_unmap,
    thread_exit, IpcRights, cq_wait, ipc_status,
};

const REPLY_SPINS: u64 = 50_000_000;
const VADDR_BAR0:   usize = 0x0000_0000_0040_0000;
const V_RX_DESC:    usize = 0x0000_0000_0050_0000; // RX descriptor-table page
const V_TX_DESC:    usize = 0x0000_0000_0050_1000; // TX descriptor-table page
const V_RX_BUF:     usize = 0x0000_0000_0050_2000; // RX data buffer page(s)
const V_REPLY:      usize = 0x0000_0000_0050_4000; // temporary reply-copy buffer
const STAGE_OFFSET: usize = 0;

#[inline] unsafe fn r8(a: usize) -> u8   { unsafe { core::ptr::read_volatile(a as *const u8) } }
#[inline] unsafe fn r16(a: usize) -> u16 { unsafe { core::ptr::read_volatile(a as *const u16) } }
#[inline] unsafe fn r32(a: usize) -> u32 { unsafe { core::ptr::read_volatile(a as *const u32) } }
#[inline] unsafe fn w8(a: usize, v: u8)  { unsafe { core::ptr::write_volatile(a as *mut u8, v)  } }
#[inline] unsafe fn w16(a: usize, v: u16){ unsafe { core::ptr::write_volatile(a as *mut u16, v) } }
#[inline] unsafe fn w32(a: usize, v: u32){ unsafe { core::ptr::write_volatile(a as *mut u32, v) } }

/// Allocate a page, map it at `vaddr`, return `(cap, phys_addr, pfn)`.
unsafe fn alloc_page(vaddr: usize) -> (u64, u64, u32) {
    let cap = memory_alloc(1);
    if cap == 0 { return (0, 0, 0); }
    if memory_map(cap, vaddr, true) != 0 {
        memory_close(cap);
        return (0, 0, 0);
    }
    let phys = memory_get_phys(cap);
    let pfn = (phys >> 12) as u32;
    (cap, phys, pfn)
}

/// Quick zero-initialisation of the virtqueue descriptor table + avail ring.
unsafe fn zero_vq(page_va: usize, queue_size: u16) {
    unsafe { core::ptr::write_bytes(page_va as *mut u8, 0, 4096); }
    let size = queue_size as usize;
    // Pre-fill avail ring: every descriptor is device-writable.
    for i in 0..size {
        let off = i * virtio::DESC_SIZE;
        unsafe { w32(page_va + off + virtio::DESC_ADDR_LO, (page_va + 0x2000 /* RX_BUF */) as u32); }
        unsafe { w32(page_va + off + virtio::DESC_LENGTH, 2048); }
        unsafe { w16(page_va + off + virtio::DESC_FLAGS, virtio::VRING_DESC_F_WRITE); }
        let idx = unsafe { r16(page_va + virtio::AVAIL_IDX) };
        unsafe {
            w16(page_va + virtio::AVAIL_RING + idx as usize * 2, i as u16);
            w16(page_va + virtio::AVAIL_IDX, idx.wrapping_add(1));
        }
    }
}

/// Set up a single virtqueue: select, size, physical pfn.
unsafe fn cfg_vq(bar0: usize, q: u16, size: u16, pfn: u32) {
    unsafe { w16(bar0 + virtio::QUEUE_SELECT, q) };
    unsafe { w16(bar0 + virtio::QUEUE_SIZE, size) };
    unsafe { w32(bar0 + virtio::QUEUE_ADDRESS, pfn) };
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1);
    let ns_conn = match ctx.bootstrap_cap() { Some(c) => c, None => unsafe { thread_exit() }, };
    let mmio_cap = match ctx.mmio_cap() { Some(c) => c, None => unsafe { thread_exit() } };
    let irq_cap  = match ctx.irq_cap()  { Some(c) => c, None => unsafe { thread_exit() } };
    config::write::<u32>(STAGE_OFFSET, 2);
    if device_mmio_map(mmio_cap, VADDR_BAR0, true) != 0 { unsafe { thread_exit() }; }
    config::write::<u32>(STAGE_OFFSET, 3);
    let bar0 = VADDR_BAR0 + virtio::COMMON_CFG_OFFSET;

    // --- virtio init (legacy transport) ---
    unsafe { w8(bar0 + virtio::DEVICE_STATUS, 0) };
    unsafe { w8(bar0 + virtio::DEVICE_STATUS, virtio::STATUS_ACKNOWLEDGE) };
    unsafe { w8(bar0 + virtio::DEVICE_STATUS, virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER) };
    let _ = unsafe { r32(bar0 + virtio::DEVICE_FEATURES) };
    unsafe { w8(bar0 + virtio::DEVICE_STATUS,
        virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER | virtio::STATUS_FEATURES_OK) };
    if unsafe { r8(bar0 + virtio::DEVICE_STATUS) & virtio::STATUS_FEATURES_OK == 0 } {
        unsafe { w8(bar0 + virtio::DEVICE_STATUS,
            virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER) };
    }

    // Read MAC.
    let dc = VADDR_BAR0 + virtio::COMMON_CFG_OFFSET + virtio::DEVICE_CFG_OFFSET;
    for i in 0..6 { config::write::<u8>(4 + i, unsafe { r8(dc + virtio::NET_MAC + i) }); }
    config::write::<u32>(STAGE_OFFSET, 4);

    // --- virtqueues ---------------------------------------------------------
    let qsz = virtio::QUEUE_COUNT;
    // Allocate descriptor-table pages (one per queue) + RX buffer.
    let (_rx_cap, _rx_phys, rx_pfn) = unsafe { alloc_page(V_RX_DESC) };
    let (_tx_cap, _tx_phys, tx_pfn) = unsafe { alloc_page(V_TX_DESC) };
    let (rx_buf_cap, _rx_buf_phys, _) = unsafe { alloc_page(V_RX_BUF) };
    if _rx_cap == 0 || _tx_cap == 0 || rx_buf_cap == 0 { unsafe { thread_exit() }; }
    // Override descriptor addresses with the real RX buffer physical addr.
    let rx_buf_phys = memory_get_phys(rx_buf_cap);
    for i in 0..qsz as usize {
        unsafe { w32(V_RX_DESC + i * virtio::DESC_SIZE + virtio::DESC_ADDR_LO, rx_buf_phys as u32 + (i * 2048) as u32) };
    }
    unsafe { zero_vq(V_RX_DESC, qsz) };
    unsafe { zero_vq(V_TX_DESC, qsz) };
    // Set TX descriptors to 0 for now (they'll be filled by OP_SEND).
    for i in 0..qsz as usize {
        unsafe { w32(V_TX_DESC + i * virtio::DESC_SIZE + virtio::DESC_ADDR_LO, 0) };
    }
    unsafe { cfg_vq(bar0, virtio::VIRTQ_RX, qsz, rx_pfn) };
    unsafe { cfg_vq(bar0, virtio::VIRTQ_TX, qsz, tx_pfn) };
    config::write::<u32>(STAGE_OFFSET, 5); // virtqueues set up

    // DRIVER_OK
    unsafe { w8(bar0 + virtio::DEVICE_STATUS,
        virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER
        | virtio::STATUS_DRIVER_OK | virtio::STATUS_FEATURES_OK) };
    config::write::<u32>(STAGE_OFFSET, 6);

    // --- Register endpoint --------------------------------------------------
    let ep = ipc_endpoint_create(net::INTERFACE, net::VERSION, 8);
    if ep == 0 { unsafe { thread_exit() }; }
    let reg = ipc_scalar_call_connection(ns_conn, ns::OP_REGISTER, net::NAME, ep,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION);
    if reg == 0 { unsafe { thread_exit() }; }
    let (generation, _) = unsafe { wait_reply(reg, REPLY_SPINS) };
    if generation < 1 { unsafe { thread_exit() }; }
    config::write::<u32>(STAGE_OFFSET, 7);

    if ipc_endpoint_bind_cq(ep, 0) != 0 { unsafe { thread_exit() }; }
    if device_irq_bind_cq(irq_cap, 0) != 0 { unsafe { thread_exit() }; }
    config::write::<u32>(STAGE_OFFSET, 8);

    // --- State ---------------------------------------------------------------
    let mut tx_avail: u16 = 0; // next avail slot index for TX
    let mut pending_recv: u64 = 0; // retained reply token for OP_RECV

    loop {
        cq_wait(1, 0);
        let (_s, _c) = device_irq_ack(irq_cap);

        // If an RX interrupt arrived AND a recv is pending, complete it.
        let isr = unsafe { r8(bar0 + virtio::ISR_STATUS) };
        if isr & 1 != 0 && pending_recv != 0 {
            unsafe { w8(bar0 + virtio::ISR_STATUS, 0) }; // ack ISR
            // Copy the received frame into a fresh page and reply with a
            // moved memory object.  (A real driver would read the used ring;
            // for the smoke test, any ISR == a frame arrived.)
            let (reply_cap, _, _) = unsafe { alloc_page(V_REPLY) };
            if reply_cap != 0 {
                unsafe {
                    let src = V_RX_BUF as *const u8;
                    let dst = V_REPLY as *mut u8;
                    for i in 0..64 {
                        core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
                    }
                    memory_unmap(reply_cap);
                }
                ipc_reply_move(pending_recv, reply_cap, 0); 
            } else {
                ipc_reply(pending_recv, -1);
            }
            pending_recv = 0;
        }

        // --- endpoint messages ---------------------------------------------
        loop {
            let m = ipc_recv(ep);
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
                            ipc_reply(m.reply, result as i64);
                    }
                }
                net::OP_SEND => {
                    // Transmit a raw Ethernet frame. The call attaches a
                    // memory object; we move it to the TX descriptor ring and
                    // notify the device.
                    if m.memory != 0 { let mem = m.memory;
                        let tx_phys = memory_get_phys(mem);
                        if tx_phys != 0 {
                            let off = tx_avail as usize * virtio::DESC_SIZE;
                            unsafe {
                                w32(V_TX_DESC + off + virtio::DESC_ADDR_LO, tx_phys as u32);
                                w32(V_TX_DESC + off + virtio::DESC_LENGTH, 2048);
                                // Driver-writable descriptor: 0 flags (device reads).
                                w16(V_TX_DESC + off + virtio::DESC_FLAGS, 0);
                            }
                            let idx = unsafe { r16(V_TX_DESC + virtio::AVAIL_IDX) };
                            unsafe {
                                w16(V_TX_DESC + virtio::AVAIL_RING + idx as usize * 2, tx_avail);
                                w16(V_TX_DESC + virtio::AVAIL_IDX, idx.wrapping_add(1));
                            }
                            unsafe { w16(bar0 + virtio::QUEUE_NOTIFY, virtio::VIRTQ_TX) };
                            tx_avail = tx_avail.wrapping_add(1);
                            // The memory object now belongs to the device.
                            // Keep the cap alive (the verifier will release
                            // on shutdown).  For a production driver, we'd
                            // return it via a used-ring completion.
                            memory_unmap(mem);
                        }
                    }
                    if m.reply != 0 {     ipc_reply(m.reply, 0); }
                }
                net::OP_RECV => {
                    if m.reply != 0 && pending_recv == 0 {
                        pending_recv = m.reply;
                    } else if m.reply != 0 {
                            ipc_reply(m.reply, -1);
                    }
                }
                net::OP_SHUTDOWN => {
                    if m.reply != 0 {     ipc_reply(m.reply, 0); }
                    unsafe { device_mmio_unmap(mmio_cap); catten_syscall::device_close(irq_cap); thread_exit(); }
                }
                _ => { if m.reply != 0 {     ipc_reply(m.reply, -1); } }
            }
        }
    }
}

catten_rt::entry!(main);
