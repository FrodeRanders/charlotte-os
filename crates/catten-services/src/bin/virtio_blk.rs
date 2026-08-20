//! virtio-blk block device driver (userspace, Phase 8/10 driver model).
//!
//! Implements the `blk0` block protocol on a modern (PCI BAR 4) virtio-blk
//! device. One split virtqueue carries the 3-descriptor request chain
//! (header → data → status); completion is polled from the used ring. The
//! borrowed data buffer is DMA-mapped per request, so the IOMMU re-maps any
//! non-contiguous physical frames into a contiguous IOVA for the device.

#![no_std]
#![no_main]

extern crate alloc;

use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    block,
    ns,
    spin_reply,
    virtio,
};
use catten_syscall::{
    ipc_status,
    DmaDirection,
    IpcRights,
    dma_map,
    dma_unmap,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_scalar_call_connection,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_size,
    memory_unmap,
    thread_exit,
};

const PAGE_SIZE: usize = 4096;
const QUEUE_SIZE: u16 = 8;

// virtio-blk request types.
const BLK_T_IN: u32 = 0;
const BLK_T_OUT: u32 = 1;
const BLK_T_FLUSH: u32 = 4;

static DMA_DOMAIN: AtomicU64 = AtomicU64::new(0);
static MMIO_BASE: AtomicU64 = AtomicU64::new(0);

#[inline]
unsafe fn w8(a: usize, v: u8) {
    unsafe { core::ptr::write_volatile(a as *mut u8, v) }
}
#[inline]
unsafe fn w16(a: usize, v: u16) {
    unsafe { core::ptr::write_volatile(a as *mut u16, v) }
}
#[inline]
unsafe fn w32(a: usize, v: u32) {
    unsafe { core::ptr::write_volatile(a as *mut u32, v) }
}
#[inline]
unsafe fn r8(a: usize) -> u8 {
    unsafe { core::ptr::read_volatile(a as *const u8) }
}
#[inline]
unsafe fn r16(a: usize) -> u16 {
    unsafe { core::ptr::read_volatile(a as *const u16) }
}
#[inline]
unsafe fn r32(a: usize) -> u32 {
    unsafe { core::ptr::read_volatile(a as *const u32) }
}
#[inline]
unsafe fn r64(a: usize) -> u64 {
    unsafe { core::ptr::read_volatile(a as *const u64) }
}
#[inline]
unsafe fn w64(a: usize, v: u64) {
    unsafe {
        w32(a, v as u32);
        w32(a + 4, (v >> 32) as u32);
    }
}

#[inline]
fn avail_offset(queue_size: u16) -> usize {
    queue_size as usize * virtio::DESC_SIZE
}

#[inline]
fn used_offset(queue_size: u16) -> usize {
    let avail_end = avail_offset(queue_size) + 6 + queue_size as usize * 2;
    avail_end.next_multiple_of(PAGE_SIZE)
}

#[inline]
fn vring_pages(queue_size: u16) -> usize {
    let used_end = used_offset(queue_size) + 6 + queue_size as usize * 8;
    used_end.div_ceil(PAGE_SIZE)
}

struct DmaBuf {
    #[allow(dead_code)]
    cap: u64,
    iova: u64,
    vaddr: usize,
}

fn alloc_dma(pages: usize) -> Option<DmaBuf> {
    let cap = memory_alloc(pages);
    if cap == 0 {
        return None;
    }
    let (map_status, vaddr) = memory_map_any(cap, true);
    if map_status != 0 {
        memory_close(cap);
        return None;
    }
    let iova = unsafe { dma_map(DMA_DOMAIN.load(Ordering::Acquire), cap, DmaDirection::Bidirectional) };
    if iova == 0 {
        memory_unmap(cap);
        memory_close(cap);
        return None;
    }
    Some(DmaBuf {
        cap,
        iova,
        vaddr,
    })
}

fn main(ctx: Context) -> ! {
    let ns_connection = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let mmio_cap = match ctx.mmio_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let _irq_cap = match ctx.irq_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let dma_domain = match config::dma_domain_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    DMA_DOMAIN.store(dma_domain, Ordering::Release);

    let (mmio_map_status, bar4) = catten_syscall::device_mmio_map_any(mmio_cap, true);
    if mmio_map_status != 0 {
        unsafe { thread_exit() };
    }
    MMIO_BASE.store(bar4 as u64, Ordering::Release);
    config::write::<u32>(4, 10);

    // --- Device init (modern transport) ---
    let common = bar4 + virtio::MODERN_COMMON;
    unsafe {
        w8(common + virtio::M_DEVICE_STATUS, 0);
        w8(common + virtio::M_DEVICE_STATUS, virtio::STATUS_ACKNOWLEDGE);
        w8(
            common + virtio::M_DEVICE_STATUS,
            virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER,
        );
        w32(common + virtio::M_DEVICE_FEATURE_SELECT, 1);
    }
    let required_features = virtio::FEATURE_VERSION_1 | virtio::FEATURE_ACCESS_PLATFORM;
    if unsafe { r32(common + virtio::M_DEVICE_FEATURE) } & required_features != required_features {
        config::write::<u32>(4, 0xe0);
        unsafe { thread_exit() };
    }
    unsafe {
        // Negotiate VIRTIO_F_VERSION_1 | VIRTIO_F_ACCESS_PLATFORM.
        w32(common + virtio::M_DRIVER_FEATURE_SELECT, 1);
        w32(common + virtio::M_DRIVER_FEATURE, required_features);
        w8(
            common + virtio::M_DEVICE_STATUS,
            virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER | virtio::STATUS_FEATURES_OK,
        );
    }
    if unsafe { r8(common + virtio::M_DEVICE_STATUS) & virtio::STATUS_FEATURES_OK == 0 } {
        config::write::<u32>(4, 0xe1);
        unsafe { thread_exit() };
    }
    config::write::<u32>(4, 11);

    // --- Capacity from device config ---
    let device = bar4 + virtio::MODERN_DEVICE;
    let capacity = unsafe { r64(device) };
    let block_size: u32 = 512;
    let total_blocks = capacity.min(u32::MAX as u64) as u32;
    if capacity == 0 || total_blocks == 0 {
        config::write::<u32>(4, 0xe3);
        unsafe { thread_exit() };
    }
    config::write::<u32>(4, 12);

    // --- Single virtqueue + request/status scratch buffers ---
    let Some(ring) = alloc_dma(vring_pages(QUEUE_SIZE)) else {
        unsafe { thread_exit() };
    };
    let Some(req) = alloc_dma(1) else {
        unsafe { thread_exit() };
    };
    unsafe {
        core::ptr::write_bytes(ring.vaddr as *mut u8, 0, vring_pages(QUEUE_SIZE) * PAGE_SIZE);
        w16(common + virtio::M_QUEUE_SELECT, 0);
        let maximum_queue_size = r16(common + virtio::M_QUEUE_SIZE);
        if maximum_queue_size < QUEUE_SIZE {
            config::write::<u32>(4, 0xe4);
            thread_exit();
        }
        w16(common + virtio::M_QUEUE_VECTOR, 0);
        w16(common + virtio::M_QUEUE_SIZE, QUEUE_SIZE);
        w64(common + virtio::M_QUEUE_DESC, ring.iova);
        w64(common + virtio::M_QUEUE_DRIVER, ring.iova + avail_offset(QUEUE_SIZE) as u64);
        w64(common + virtio::M_QUEUE_DEVICE, ring.iova + used_offset(QUEUE_SIZE) as u64);
        w16(common + virtio::M_QUEUE_ENABLE, 1);
        w8(
            common + virtio::M_DEVICE_STATUS,
            virtio::STATUS_ACKNOWLEDGE
                | virtio::STATUS_DRIVER
                | virtio::STATUS_FEATURES_OK
                | virtio::STATUS_DRIVER_OK,
        );
    }
    let notify_offset = unsafe { r16(common + virtio::M_QUEUE_NOTIFY_OFF) };
    let notify = bar4 + virtio::MODERN_NOTIFY + notify_offset as usize * virtio::MODERN_NOTIFY_MULTIPLIER;
    config::write::<u32>(4, 13);

    // Pre-build the descriptor chain (header@0 → data@1 → status@2). Only the
    // data descriptor's address/length/flags change per request.
    let desc: usize = ring.vaddr;
    unsafe {
        // Descriptor 0: request header (device reads).
        w32(desc + 0 * virtio::DESC_SIZE + virtio::DESC_ADDR_LO, req.iova as u32);
        w32(desc + 0 * virtio::DESC_SIZE + virtio::DESC_ADDR_HI, (req.iova >> 32) as u32);
        w32(desc + 0 * virtio::DESC_SIZE + virtio::DESC_LENGTH, 16);
        w16(
            desc + 0 * virtio::DESC_SIZE + virtio::DESC_FLAGS,
            virtio::VRING_DESC_F_NEXT,
        );
        w16(desc + 0 * virtio::DESC_SIZE + virtio::DESC_NEXT, 1);
        // Descriptor 2: status byte (device writes).
        let status_iova = req.iova.checked_add(16).expect("virtio status IOVA overflow");
        w32(desc + 2 * virtio::DESC_SIZE + virtio::DESC_ADDR_LO, status_iova as u32);
        w32(
            desc + 2 * virtio::DESC_SIZE + virtio::DESC_ADDR_HI,
            (status_iova >> 32) as u32,
        );
        w32(desc + 2 * virtio::DESC_SIZE + virtio::DESC_LENGTH, 1);
        w16(
            desc + 2 * virtio::DESC_SIZE + virtio::DESC_FLAGS,
            virtio::VRING_DESC_F_WRITE,
        );
        w16(desc + 2 * virtio::DESC_SIZE + virtio::DESC_NEXT, 0);
    }

    let avail_ring: usize = ring.vaddr + avail_offset(QUEUE_SIZE) + virtio::AVAIL_RING;
    let avail_idx: usize = ring.vaddr + avail_offset(QUEUE_SIZE) + virtio::AVAIL_IDX;
    let used_idx: usize = ring.vaddr + used_offset(QUEUE_SIZE) + virtio::USED_IDX;
    let mut avail_pos: u16 = 0;

    // --- Register with the name service ---
    let endpoint = ipc_endpoint_create(block::INTERFACE, block::VERSION, 64);
    if endpoint == 0 {
        unsafe { thread_exit() };
    }
    let register = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_REGISTER,
        block::NAME,
        endpoint,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if register == 0 {
        unsafe { thread_exit() };
    }
    let (generation, _) = spin_reply(register);
    if generation < 1 {
        unsafe { thread_exit() };
    }
    ipc_endpoint_bind_cq(endpoint, 0);
    config::write::<u32>(4, 14);
    config::write::<u32>(20, 0x900d);

    // Build a request in the shared chain and return its completion status.
    let submit = |req_type: u32,
                  sector: u64,
                  data_iova: u64,
                  data_len: u32,
                  avail_pos: &mut u16|
     -> bool {
        unsafe {
            // Header: type + reserved + sector.
            let hdr = req.vaddr as *mut u8;
            w32(hdr.add(0) as usize, req_type);
            w32(hdr.add(4) as usize, 0);
            w64(hdr.add(8) as usize, sector);
            w8(hdr.add(16) as usize, 0xff);
            // The FLUSH request carries no data descriptor; the header chains
            // straight to the status byte.
            if data_len == 0 {
                w16(desc + 0 * virtio::DESC_SIZE + virtio::DESC_NEXT, 2);
            } else {
                w16(desc + 0 * virtio::DESC_SIZE + virtio::DESC_NEXT, 1);
                let d = desc + 1 * virtio::DESC_SIZE;
                w32(d + virtio::DESC_ADDR_LO, data_iova as u32);
                w32(d + virtio::DESC_ADDR_HI, (data_iova >> 32) as u32);
                w32(d + virtio::DESC_LENGTH, data_len);
                let flags = if req_type == BLK_T_IN {
                    virtio::VRING_DESC_F_NEXT | virtio::VRING_DESC_F_WRITE
                } else {
                    virtio::VRING_DESC_F_NEXT
                };
                w16(d + virtio::DESC_FLAGS, flags);
                w16(d + virtio::DESC_NEXT, 2);
            }
            // Publish the chain head and kick.
            let slot = *avail_pos as usize % QUEUE_SIZE as usize;
            w16(avail_ring + slot * 2, 0);
            *avail_pos = avail_pos.wrapping_add(1);
            core::sync::atomic::fence(Ordering::Release);
            w16(avail_idx, *avail_pos);
            w32(notify, 0);
            // Poll the used ring for this request.
            let mut completed = false;
            for _ in 0..1_000_000 {
                if r16(used_idx) == *avail_pos {
                    completed = true;
                    break;
                }
                core::hint::spin_loop();
            }
            core::sync::atomic::fence(Ordering::Acquire);
            completed && r8(req.vaddr as usize + 16) == 0
        }
    };

    loop {
        let message = ipc_recv(endpoint);
        if message.status == ipc_status::NO_MESSAGE {
            catten_syscall::cq_wait(1, 0);
            continue;
        }
        if message.status == ipc_status::ENDPOINT_CLOSED {
            unsafe { thread_exit() };
        }
        if !message.is_ok() {
            continue;
        }
        match message.opcode {
            block::OP_INFO => {
                if message.reply != 0 {
                    ipc_reply(
                        message.reply,
                        ((block_size as u64) | ((total_blocks as u64) << 32)) as i64,
                    );
                }
            }
            block::OP_READ => {
                let (lba, count) = charlotte_protocol_block::unpack_lba_count(message.arg0);
                let Some(transfer_bytes) = (count as u64).checked_mul(block_size as u64) else {
                    ipc_reply(message.reply, block::ERR_INVALID_RANGE);
                    continue;
                };
                if count == 0
                    || transfer_bytes == 0
                    || transfer_bytes > u32::MAX as u64
                    || message.memory == 0
                    || transfer_bytes > memory_size(message.memory) as u64
                    || lba
                        .checked_add(count as u64)
                        .is_none_or(|end| end > total_blocks as u64)
                {
                    ipc_reply(message.reply, block::ERR_INVALID_RANGE);
                    continue;
                }
                let iova = unsafe { dma_map(dma_domain, message.memory, DmaDirection::DeviceWrite) };
                if iova == 0 {
                    ipc_reply(message.reply, block::ERR_IO_ERROR);
                    continue;
                }
                let ok = submit(BLK_T_IN, lba, iova, transfer_bytes as u32, &mut avail_pos);
                dma_unmap(dma_domain, iova);
                ipc_reply(message.reply, if ok { block::ERR_OK } else { block::ERR_IO_ERROR });
            }
            block::OP_WRITE => {
                let (lba, count) = charlotte_protocol_block::unpack_lba_count(message.arg0);
                let Some(transfer_bytes) = (count as u64).checked_mul(block_size as u64) else {
                    ipc_reply(message.reply, block::ERR_INVALID_RANGE);
                    continue;
                };
                if count == 0
                    || transfer_bytes == 0
                    || transfer_bytes > u32::MAX as u64
                    || message.memory == 0
                    || transfer_bytes > memory_size(message.memory) as u64
                    || lba
                        .checked_add(count as u64)
                        .is_none_or(|end| end > total_blocks as u64)
                {
                    ipc_reply(message.reply, block::ERR_INVALID_RANGE);
                    continue;
                }
                let iova = unsafe { dma_map(dma_domain, message.memory, DmaDirection::DeviceRead) };
                if iova == 0 {
                    ipc_reply(message.reply, block::ERR_IO_ERROR);
                    continue;
                }
                let ok = submit(BLK_T_OUT, lba, iova, transfer_bytes as u32, &mut avail_pos);
                dma_unmap(dma_domain, iova);
                ipc_reply(message.reply, if ok { block::ERR_OK } else { block::ERR_IO_ERROR });
            }
            block::OP_FLUSH => {
                let ok = submit(BLK_T_FLUSH, 0, 0, 0, &mut avail_pos);
                ipc_reply(message.reply, if ok { block::ERR_OK } else { block::ERR_IO_ERROR });
            }
            _ => {
                if message.reply != 0 {
                    ipc_reply(message.reply, -1);
                }
            }
        }
    }
}

catten_rt::entry!(main);
