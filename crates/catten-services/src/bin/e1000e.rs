//! Intel 82574L/E1000E userspace Ethernet driver.
//!
//! This backend serves the same hardware-neutral `net0` frame protocol as the
//! virtio-net driver. It is intentionally limited to the single-queue legacy
//! descriptor mode needed by VMware's E1000E virtual NIC and QEMU's 82574L
//! model: 2-KiB receive buffers, one receive ring, one transmit ring, no
//! checksum or segmentation offload, and MSI-X vector zero with bounded
//! polling as a lost-interrupt fallback.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    collections::VecDeque,
    vec,
};
#[cfg(target_arch = "aarch64")]
use core::arch::asm;

use catten_rt::{
    Context,
    ShutdownRequest,
    config,
};
use catten_services::{
    net,
    ns,
    wait_reply,
};
use catten_syscall::{
    DmaDirection,
    IpcRights,
    cq_wait_timeout,
    device_irq_ack,
    device_irq_bind_cq,
    device_mmio_map_any,
    device_mmio_unmap,
    dma_map,
    ipc_close,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_scalar_call_connection,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
    thread_exit,
};
use charlotte_launch::net_status as status;

const LINK_WAIT_ATTEMPTS: usize = 500;
const LINK_WAIT_MILLIS: u64 = 10;
const RESET_WAIT_ATTEMPTS: usize = 1_000;
const RESET_WAIT_MILLIS: u64 = 1;
const PAGE_SIZE: usize = 4096;
const DESCRIPTOR_SIZE: usize = 16;
const RING_SIZE: usize = 128;
const BUFFER_SIZE: usize = 2048;
const MAX_FRAME_SIZE: usize = 1514;

/// Monotonic reactor-tick counter for periodic heartbeat logging.
static HEARTBEAT_TICKS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

// 82574L register offsets.
const CTRL: usize = 0x0000;
const STATUS: usize = 0x0008;
const CTRL_EXT: usize = 0x0018;
const ICR: usize = 0x00c0;
const IMS: usize = 0x00d0;
const IMC: usize = 0x00d8;
const IVAR: usize = 0x00e4;
const RCTL: usize = 0x0100;
const TCTL: usize = 0x0400;
const TIPG: usize = 0x0410;
const RDBAL: usize = 0x2800;
const RDBAH: usize = 0x2804;
const RDLEN: usize = 0x2808;
const SRRCTL: usize = 0x280c;
const RDH: usize = 0x2810;
const RDT: usize = 0x2818;
const RXDCTL: usize = 0x2828;
const TDBAL: usize = 0x3800;
const TDBAH: usize = 0x3804;
const TDLEN: usize = 0x3808;
const TDH: usize = 0x3810;
const TDT: usize = 0x3818;
const TXDCTL: usize = 0x3828;
const RAL0: usize = 0x5400;
const RAH0: usize = 0x5404;

const CTRL_SLU: u32 = 1 << 6;
const CTRL_RST: u32 = 1 << 26;
const STATUS_LU: u32 = 1 << 1;
const RCTL_EN: u32 = 1 << 1;
const RCTL_BAM: u32 = 1 << 15;
const RCTL_SECRC: u32 = 1 << 26;
const TCTL_EN: u32 = 1 << 1;
const TCTL_PSP: u32 = 1 << 3;
const QUEUE_ENABLE: u32 = 1 << 25;
const SRRCTL_DROP_EN: u32 = 1 << 31;

const RXD_STAT_DD: u8 = 1 << 0;
const RXD_STAT_EOP: u8 = 1 << 1;
const TXD_STAT_DD: u8 = 1 << 0;
const TXD_CMD_EOP: u32 = 1 << 24;
const TXD_CMD_IFCS: u32 = 1 << 25;
const TXD_CMD_RS: u32 = 1 << 27;

const ICR_TXDW: u32 = 1 << 0;
const ICR_LSC: u32 = 1 << 2;
const ICR_RXT0: u32 = 1 << 7;
const ICR_RXQ0: u32 = 1 << 20;
const ICR_TXQ0: u32 = 1 << 22;
const ICR_OTHER: u32 = 1 << 24;

static MMIO_BASE: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[inline]
fn dma_write_barrier() {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        asm!("dmb oshst", options(nostack, preserves_flags))
    }
    #[cfg(not(target_arch = "aarch64"))]
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release)
}

#[inline]
fn dma_read_barrier() {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        asm!("dmb oshld", options(nostack, preserves_flags))
    }
    #[cfg(not(target_arch = "aarch64"))]
    core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire)
}

#[inline]
unsafe fn mmio_read(offset: usize) -> u32 {
    unsafe {
        core::ptr::read_volatile(
            (MMIO_BASE.load(core::sync::atomic::Ordering::Relaxed) + offset) as *const u32,
        )
    }
}

#[inline]
unsafe fn mmio_write(offset: usize, value: u32) {
    unsafe {
        core::ptr::write_volatile(
            (MMIO_BASE.load(core::sync::atomic::Ordering::Relaxed) + offset) as *mut u32,
            value,
        )
    }
}

#[inline]
unsafe fn desc_read_u8(base: usize, slot: usize, offset: usize) -> u8 {
    unsafe { core::ptr::read_volatile((base + slot * DESCRIPTOR_SIZE + offset) as *const u8) }
}

#[inline]
unsafe fn desc_read_u16(base: usize, slot: usize, offset: usize) -> u16 {
    unsafe { core::ptr::read_volatile((base + slot * DESCRIPTOR_SIZE + offset) as *const u16) }
}

#[inline]
unsafe fn desc_write_u32(base: usize, slot: usize, offset: usize, value: u32) {
    unsafe {
        core::ptr::write_volatile((base + slot * DESCRIPTOR_SIZE + offset) as *mut u32, value)
    }
}

#[inline]
unsafe fn desc_write_u64(base: usize, slot: usize, offset: usize, value: u64) {
    unsafe {
        core::ptr::write_volatile((base + slot * DESCRIPTOR_SIZE + offset) as *mut u64, value)
    }
}

/// Allocate, map, and pin one DMA object for the lifetime of this process.
unsafe fn alloc_dma(dma_domain: u64, pages: usize, direction: DmaDirection) -> (u64, u64, usize) {
    let cap = memory_alloc(pages);
    if cap == 0 {
        return (0, 0, 0);
    }
    let (map_status, vaddr) = memory_map_any(cap, true);
    if map_status != 0 {
        memory_close(cap);
        return (0, 0, 0);
    }
    let iova = unsafe { dma_map(dma_domain, cap, direction) };
    if iova == 0 {
        memory_unmap(cap);
        memory_close(cap);
        return (0, 0, 0);
    }
    (cap, iova, vaddr)
}

struct ReceivedFrame {
    cap: u64,
    len: usize,
}

fn deliver_received(
    queue: &mut VecDeque<ReceivedFrame>,
    pending_recv: &mut u64,
    delivered: &mut u16,
    delivery_error: &mut u32,
) {
    if *pending_recv == 0 {
        return;
    }
    let Some(frame) = queue.pop_front() else {
        return;
    };
    let status = ipc_reply_move(*pending_recv, frame.cap, frame.len as i64);
    if status != 0 {
        memory_close(frame.cap);
        *delivery_error = status as u32;
    } else {
        *delivered = delivered.wrapping_add(1);
    }
    *pending_recv = 0;
}

unsafe fn reset_controller() -> bool {
    unsafe {
        mmio_write(IMC, u32::MAX);
        let _ = mmio_read(ICR);
        let ctrl = mmio_read(CTRL);
        mmio_write(CTRL, ctrl | CTRL_RST);
    }
    for _ in 0..RESET_WAIT_ATTEMPTS {
        if unsafe { mmio_read(CTRL) } & CTRL_RST == 0 {
            unsafe {
                mmio_write(IMC, u32::MAX);
                let _ = mmio_read(ICR);
            }
            return true;
        }
        // Reset completion is asynchronous. Let another runnable thread use
        // this LP between bounded status checks instead of burning cycles.
        let _ = cq_wait_timeout(1, RESET_WAIT_MILLIS, 0);
    }
    false
}

unsafe fn read_mac() -> [u8; 6] {
    let low = unsafe { mmio_read(RAL0) };
    let high = unsafe { mmio_read(RAH0) };
    [
        low as u8,
        (low >> 8) as u8,
        (low >> 16) as u8,
        (low >> 24) as u8,
        high as u8,
        (high >> 8) as u8,
    ]
}

unsafe fn drain_tx(tx_ring: usize, in_use: &mut [bool], completed: &mut u16) {
    for (slot, busy) in in_use.iter_mut().enumerate() {
        if *busy && unsafe { desc_read_u8(tx_ring, slot, 12) } & TXD_STAT_DD != 0 {
            dma_read_barrier();
            *busy = false;
            *completed = completed.wrapping_add(1);
        }
    }
}

unsafe fn drain_rx(
    rx_ring: usize,
    rx_buffers: usize,
    next: &mut usize,
    completed: &mut u16,
    accepted: &mut u16,
    queue: &mut VecDeque<ReceivedFrame>,
) {
    loop {
        let status = unsafe { desc_read_u8(rx_ring, *next, 12) };
        if status & RXD_STAT_DD == 0 {
            break;
        }
        dma_read_barrier();
        let errors = unsafe { desc_read_u8(rx_ring, *next, 13) };
        let len = unsafe { desc_read_u16(rx_ring, *next, 8) } as usize;
        config::write::<u8>(status::LAST_RX_DESCRIPTOR_STATUS, status);
        config::write::<u8>(status::LAST_RX_DESCRIPTOR_ERRORS, errors);
        config::write::<u16>(status::LAST_RX_DESCRIPTOR_LENGTH, len as u16);
        if status & RXD_STAT_EOP != 0
            && errors == 0
            && (14..=MAX_FRAME_SIZE).contains(&len)
            && queue.len() < RING_SIZE
        {
            let cap = memory_alloc(1);
            if cap != 0 {
                let (map_status, vaddr) = memory_map_any(cap, true);
                if map_status == 0 {
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            (rx_buffers + *next * BUFFER_SIZE) as *const u8,
                            vaddr as *mut u8,
                            len,
                        )
                    };
                    memory_unmap(cap);
                    queue.push_back(ReceivedFrame {
                        cap,
                        len,
                    });
                    *accepted = accepted.wrapping_add(1);
                } else {
                    memory_close(cap);
                }
            }
        }

        // Clearing the write-back half returns this legacy descriptor to the
        // device after RDT advances past it.
        unsafe { desc_write_u64(rx_ring, *next, 8, 0) };
        dma_write_barrier();
        let returned = *next;
        *next = (*next + 1) % RING_SIZE;
        *completed = completed.wrapping_add(1);
        unsafe { mmio_write(RDT, returned as u32) };
    }
}

fn serve(ctx: &Context) -> ShutdownRequest {
    config::write::<u32>(status::STAGE, 1);
    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let mmio_cap = match ctx.mmio_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let irq_cap = match ctx.irq_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let dma_domain = match config::dma_domain_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 2);

    let (map_status, mmio_base) = device_mmio_map_any(mmio_cap, true);
    if map_status != 0 {
        unsafe { thread_exit() };
    }
    MMIO_BASE.store(mmio_base, core::sync::atomic::Ordering::Relaxed);
    config::write::<u32>(status::STAGE, 3);

    if !unsafe { reset_controller() } {
        config::write::<u32>(status::TX_PROGRESS, 0xe001);
        unsafe { thread_exit() };
    }
    let mac = unsafe { read_mac() };
    if mac == [0; 6] {
        config::write::<u32>(status::TX_PROGRESS, 0xe002);
        unsafe { thread_exit() };
    }
    for (index, byte) in mac.iter().enumerate() {
        config::write::<u8>(status::MAC + index, *byte);
    }
    config::write::<u32>(status::STAGE, 4);

    let ring_pages = (RING_SIZE * DESCRIPTOR_SIZE).div_ceil(PAGE_SIZE);
    let buffer_pages = (RING_SIZE * BUFFER_SIZE).div_ceil(PAGE_SIZE);
    let (rx_ring_cap, rx_ring_iova, rx_ring) =
        unsafe { alloc_dma(dma_domain, ring_pages, DmaDirection::Bidirectional) };
    let (tx_ring_cap, tx_ring_iova, tx_ring) =
        unsafe { alloc_dma(dma_domain, ring_pages, DmaDirection::Bidirectional) };
    let (rx_buffer_cap, rx_buffer_iova, rx_buffers) =
        unsafe { alloc_dma(dma_domain, buffer_pages, DmaDirection::DeviceWrite) };
    let (tx_buffer_cap, tx_buffer_iova, tx_buffers) =
        unsafe { alloc_dma(dma_domain, buffer_pages, DmaDirection::DeviceRead) };
    if rx_ring_cap == 0 || tx_ring_cap == 0 || rx_buffer_cap == 0 || tx_buffer_cap == 0 {
        config::write::<u32>(status::TX_PROGRESS, 0xe003);
        unsafe { thread_exit() };
    }
    unsafe {
        core::ptr::write_bytes(rx_ring as *mut u8, 0, ring_pages * PAGE_SIZE);
        core::ptr::write_bytes(tx_ring as *mut u8, 0, ring_pages * PAGE_SIZE);
        for slot in 0..RING_SIZE {
            desc_write_u64(rx_ring, slot, 0, rx_buffer_iova + (slot * BUFFER_SIZE) as u64);
            desc_write_u64(tx_ring, slot, 0, tx_buffer_iova + (slot * BUFFER_SIZE) as u64);
            // An idle transmit descriptor is initially complete.
            desc_write_u32(tx_ring, slot, 12, TXD_STAT_DD as u32);
        }
    }
    dma_write_barrier();

    unsafe {
        mmio_write(RCTL, 0);
        mmio_write(TCTL, 0);
        mmio_write(RDBAL, rx_ring_iova as u32);
        mmio_write(RDBAH, (rx_ring_iova >> 32) as u32);
        mmio_write(RDLEN, (RING_SIZE * DESCRIPTOR_SIZE) as u32);
        mmio_write(RDH, 0);
        mmio_write(RDT, (RING_SIZE - 1) as u32);
        // 2-KiB packet buffers, legacy descriptors, drop when no descriptor.
        mmio_write(SRRCTL, 2 | SRRCTL_DROP_EN);
        mmio_write(RXDCTL, QUEUE_ENABLE | 8 | (8 << 8) | (4 << 16));

        mmio_write(TDBAL, tx_ring_iova as u32);
        mmio_write(TDBAH, (tx_ring_iova >> 32) as u32);
        mmio_write(TDLEN, (RING_SIZE * DESCRIPTOR_SIZE) as u32);
        mmio_write(TDH, 0);
        mmio_write(TDT, 0);
        mmio_write(TXDCTL, QUEUE_ENABLE | 8 | (8 << 8) | (4 << 16));
        mmio_write(TIPG, 10 | (8 << 10) | (6 << 20));

        let ctrl = mmio_read(CTRL);
        mmio_write(CTRL, ctrl | CTRL_SLU);
        // Collision threshold 15 and full-duplex collision distance 64.
        mmio_write(TCTL, TCTL_EN | TCTL_PSP | (15 << 4) | (64 << 12));
        mmio_write(RCTL, RCTL_EN | RCTL_BAM | RCTL_SECRC);
    }
    config::write::<u32>(status::RX_RING_PFN, (rx_ring_iova >> 12) as u32);
    config::write::<u32>(status::TX_RING_PFN, (tx_ring_iova >> 12) as u32);
    config::write::<u16>(status::RX_QUEUE_ENABLED, 1);
    config::write::<u16>(status::TX_QUEUE_ENABLED, 1);
    config::write::<u16>(status::RX_QUEUE_SIZE, RING_SIZE as u16);
    config::write::<u32>(status::STAGE, 6);

    // The 82574 reports link down for a short period while its emulated PHY
    // negotiates. Do not publish net0 until that transition has completed:
    // early consumers treat a down link as a permanent driver failure. Bind
    // the device interrupt first so the wait yields instead of monopolising
    // the boot LP, while bounded polling still covers a missed link interrupt.
    if device_irq_bind_cq(irq_cap, 0) != 0 {
        config::write::<u32>(status::TX_PROGRESS, 0xe004);
        unsafe { thread_exit() };
    }
    unsafe {
        mmio_write(IVAR, 0x0008_0808);
        let ctrl_ext = mmio_read(CTRL_EXT);
        mmio_write(CTRL_EXT, ctrl_ext & !((1 << 24) | (1 << 27)));
        let _ = mmio_read(ICR);
        mmio_write(IMS, ICR_TXDW | ICR_LSC | ICR_RXT0 | ICR_RXQ0 | ICR_TXQ0 | ICR_OTHER);
    }
    let mut link_up = false;
    for _ in 0..LINK_WAIT_ATTEMPTS {
        if unsafe { mmio_read(STATUS) } & STATUS_LU != 0 {
            link_up = true;
            break;
        }
        let _ = cq_wait_timeout(1, LINK_WAIT_MILLIS, 0);
        let cause = unsafe { mmio_read(ICR) };
        let _ = device_irq_ack(irq_cap);
        config::write::<u32>(status::INTERRUPT_CAUSE, cause);
    }
    if !link_up {
        config::write::<u32>(status::TX_PROGRESS, 0xe005);
        unsafe { thread_exit() };
    }
    config::write::<u16>(status::LINK, 1);
    config::write::<u32>(status::STAGE, 7);

    let ep = ipc_endpoint_create(net::INTERFACE, net::VERSION, 8);
    if ep == 0 {
        unsafe { thread_exit() };
    }
    let registration = ipc_scalar_call_connection(
        ns_conn,
        ns::OP_REGISTER,
        net::NAME,
        ep,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if registration == 0 {
        unsafe { thread_exit() };
    }
    let (generation, _) = unsafe { wait_reply(registration, 0) };
    if generation < 1 || ipc_endpoint_bind_cq(ep, 0) != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 8);

    let mut tx_next = 0usize;
    let mut rx_next = 0usize;
    let mut tx_completed = 0u16;
    let mut rx_completed = 0u16;
    let mut rx_accepted = 0u16;
    let mut rx_delivered = 0u16;
    let mut rx_delivery_error = 0u32;
    let mut tx_in_use = vec![false; RING_SIZE];
    let mut received: VecDeque<ReceivedFrame> = VecDeque::new();
    let mut pending_recv = 0u64;

    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            let endpoint_closed = ipc_close(ep) == ipc_status::OK;
            if pending_recv != 0 {
                ipc_close(pending_recv);
            }
            for frame in received.drain(..) {
                memory_close(frame.cap);
            }
            let tx_drained = (0..1_000_000).any(|_| {
                unsafe { drain_tx(tx_ring, &mut tx_in_use, &mut tx_completed) };
                if tx_in_use.iter().all(|in_use| !*in_use) {
                    true
                } else {
                    core::hint::spin_loop();
                    false
                }
            });
            // The global controller reset masks interrupts and disables both
            // descriptor engines. Issue it even after a drain timeout to stop
            // DMA, but publish graceful quiescence only if the queue drained.
            let reset = unsafe { reset_controller() };
            if endpoint_closed && tx_drained && reset && catten_syscall::device_close(irq_cap) == 0
            {
                return request;
            }
            catten_rt::logln!("[e1000e] device quiescence failed; retaining controller domain");
            loop {
                catten_services::sleep_ms(100);
            }
        }
        let _ = cq_wait_timeout(1, 10, 0);
        let cause = unsafe { mmio_read(ICR) };
        let (_status, _count) = device_irq_ack(irq_cap);
        config::write::<u32>(status::INTERRUPT_CAUSE, cause);
        unsafe {
            drain_tx(tx_ring, &mut tx_in_use, &mut tx_completed);
            drain_rx(
                rx_ring,
                rx_buffers,
                &mut rx_next,
                &mut rx_completed,
                &mut rx_accepted,
                &mut received,
            );
        }
        config::write::<u16>(status::RX_USED_SEEN, rx_completed);
        config::write::<u16>(status::TX_USED_SEEN, tx_completed);
        config::write::<u16>(status::RX_UNRECYCLED, received.len().min(u16::MAX as usize) as u16);
        config::write::<u16>(status::RX_ACCEPTED, rx_accepted);
        config::write::<u16>(status::RX_DELIVERED, rx_delivered);
        config::write::<u32>(status::RX_DELIVERY_ERROR, rx_delivery_error);
        // Periodic heartbeat (~every 1024 iterations): the interrupt-cause
        // register plus ring health, to expose an interrupt storm or a wedged
        // descriptor ring while the rest of the stack is frozen.
        let tick = HEARTBEAT_TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if tick & 0x3ff == 0 {
            let tx_busy = tx_in_use.iter().filter(|u| **u).count();
            catten_rt::logln!(
                "[e1000e] hb rx_pending={} tx_busy={}/{} icr={:#x}",
                received.len(),
                tx_busy,
                RING_SIZE,
                cause
            );
        }
        deliver_received(
            &mut received,
            &mut pending_recv,
            &mut rx_delivered,
            &mut rx_delivery_error,
        );

        loop {
            let message = ipc_recv(ep);
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
                        let link = (unsafe { mmio_read(STATUS) } & STATUS_LU != 0) as u64;
                        let result = link
                            | ((mac[0] as u64) << 8)
                            | ((mac[1] as u64) << 16)
                            | ((mac[2] as u64) << 24)
                            | ((mac[3] as u64) << 32)
                            | ((mac[4] as u64) << 40)
                            | ((mac[5] as u64) << 48);
                        config::write::<u16>(status::LINK, link as u16);
                        ipc_reply(message.reply, result as i64);
                    }
                }
                net::OP_SEND => {
                    let frame_len = message.arg0 as usize;
                    unsafe { drain_tx(tx_ring, &mut tx_in_use, &mut tx_completed) };
                    let slot = tx_next;
                    if message.memory == 0
                        || !(14..=MAX_FRAME_SIZE).contains(&frame_len)
                        || tx_in_use[slot]
                    {
                        if message.memory != 0 {
                            memory_close(message.memory);
                        }
                        if message.reply != 0 {
                            ipc_reply(message.reply, -1);
                        }
                        continue;
                    }
                    let (input_status, input_vaddr) = memory_map_any(message.memory, false);
                    if input_status != 0 {
                        memory_close(message.memory);
                        if message.reply != 0 {
                            ipc_reply(message.reply, -1);
                        }
                        continue;
                    }
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            input_vaddr as *const u8,
                            (tx_buffers + slot * BUFFER_SIZE) as *mut u8,
                            frame_len,
                        )
                    };
                    memory_unmap(message.memory);
                    memory_close(message.memory);

                    unsafe {
                        desc_write_u32(
                            tx_ring,
                            slot,
                            8,
                            frame_len as u32 | TXD_CMD_EOP | TXD_CMD_IFCS | TXD_CMD_RS,
                        );
                        desc_write_u32(tx_ring, slot, 12, 0);
                    }
                    tx_in_use[slot] = true;
                    dma_write_barrier();
                    tx_next = (tx_next + 1) % RING_SIZE;
                    unsafe { mmio_write(TDT, tx_next as u32) };
                    config::write::<u32>(status::TX_PROGRESS, 4);
                    if message.reply != 0 {
                        ipc_reply(message.reply, 0);
                    }
                }
                net::OP_RECV => {
                    if message.reply != 0 && pending_recv == 0 {
                        pending_recv = message.reply;
                        deliver_received(
                            &mut received,
                            &mut pending_recv,
                            &mut rx_delivered,
                            &mut rx_delivery_error,
                        );
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }
                net::OP_SHUTDOWN => {
                    unsafe {
                        mmio_write(IMC, u32::MAX);
                        mmio_write(RCTL, 0);
                        mmio_write(TCTL, 0);
                    }
                    if message.reply != 0 {
                        ipc_reply(message.reply, 0);
                    }
                    unsafe {
                        device_mmio_unmap(mmio_cap);
                        catten_syscall::device_close(irq_cap);
                        thread_exit();
                    }
                }
                _ => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }
            }
        }
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete_device_quiesced()
}

catten_rt::entry!(main);
