//! AHCI/SATA block device driver (userspace, Phase 8/10 driver model).
//!
//! Implements the `blk0` block protocol on top of an AHCI Host Bus Adapter.
//! Unlike the NVMe driver this uses a single command slot with synchronous
//! (polled) completion: the borrowed data buffer is DMA-mapped into the
//! device's IOMMU domain, a register Host-to-Device FIS + one PRDT entry are
//! built, the command is issued through `PxCI`, and completion is observed by
//! polling the slot clear. The IOMMU exposes the memory object as a contiguous
//! IOVA range, so a single PRDT entry covers the whole transfer.

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
    memory_unmap,
    thread_exit,
};

const PAGE_SIZE: usize = 4096;

// HBA MMIO registers.
const HBA_GHC: usize = 0x04;
const HBA_PI: usize = 0x0c;
const PORT_REGS: usize = 0x100;
const PORT_STRIDE: usize = 0x80;

// Port registers (offsets within a port register block).
const PXCLB: usize = 0x00;
const PXFB: usize = 0x08;
const PXIS: usize = 0x10;
const PXIE: usize = 0x14;
const PXCMD: usize = 0x18;
const PXTFD: usize = 0x20;
const PXSSTS: usize = 0x28;
const PXSERR: usize = 0x30;
const PXCI: usize = 0x38;

// GHC bits.
const GHC_AE: u32 = 1 << 31;
// PxCMD bits.
const PXCMD_ST: u32 = 1 << 0;
const PXCMD_FRE: u32 = 1 << 4;
const PXCMD_FR: u32 = 1 << 14;
const PXCMD_CR: u32 = 1 << 15;
// PxTFD bits.
const PXTFD_BSY: u8 = 1 << 7;
const PXTFD_DRQ: u8 = 1 << 3;
const PXTFD_ERR: u8 = 1 << 0;

// ATA commands.
const ATA_READ_DMA_EXT: u8 = 0x25;
const ATA_WRITE_DMA_EXT: u8 = 0x35;
const ATA_FLUSH_CACHE: u8 = 0xe7;
const ATA_IDENTIFY: u8 = 0xec;

const FIS_TYPE_REG_H2D: u8 = 0x27;

// Command header DW0 fields.
const CMDHDR_CFL: u32 = 5; // 5 DWs for the register H2D FIS
const CMDHDR_WRITE: u32 = 1 << 6; // device writes to host memory (disk READ)
const CMDHDR_PRDTL: u32 = 1 << 16;

const PRDT_OFFSET: usize = 0x80;

static DMA_DOMAIN: AtomicU64 = AtomicU64::new(0);
static HBA_BASE: AtomicU64 = AtomicU64::new(0);

struct QueueMemory {
    #[allow(dead_code)]
    cap: u64,
    iova: u64,
    vaddr: usize,
}

fn alloc_dma_buffer(bytes: usize) -> Option<QueueMemory> {
    let pages = bytes.div_ceil(PAGE_SIZE).max(1);
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
    Some(QueueMemory {
        cap,
        iova,
        vaddr,
    })
}

fn read_mmio32(offset: usize) -> u32 {
    unsafe { ((HBA_BASE.load(Ordering::Acquire) as usize + offset) as *const u32).read_volatile() }
}

fn write_mmio32(offset: usize, value: u32) {
    unsafe { ((HBA_BASE.load(Ordering::Acquire) as usize + offset) as *mut u32).write_volatile(value) }
}

fn port_base(port: usize) -> usize {
    PORT_REGS + port * PORT_STRIDE
}

fn write_fis(cmd_table: &QueueMemory, command: u8, lba: u64, count: u16) {
    let fis = cmd_table.vaddr as *mut u8;
    unsafe {
        fis.add(0).write_volatile(FIS_TYPE_REG_H2D);
        fis.add(1).write_volatile(0x80); // C = command
        fis.add(2).write_volatile(command);
        fis.add(3).write_volatile(0); // features[7:0]
        fis.add(4).write_volatile(lba as u8);
        fis.add(5).write_volatile((lba >> 8) as u8);
        fis.add(6).write_volatile((lba >> 16) as u8);
        fis.add(7).write_volatile(0xe0 | ((lba >> 24) as u8 & 0x0f)); // device (LBA, master)
        fis.add(8).write_volatile((lba >> 24) as u8);
        fis.add(9).write_volatile((lba >> 32) as u8);
        fis.add(10).write_volatile((lba >> 40) as u8);
        fis.add(11).write_volatile(0); // features[15:8]
        fis.add(12).write_volatile(count as u8);
        fis.add(13).write_volatile((count >> 8) as u8);
    }
}

fn write_prdt(cmd_table: &QueueMemory, iova: u64, byte_count: u32) {
    let prdt = (cmd_table.vaddr + PRDT_OFFSET) as *mut u32;
    unsafe {
        prdt.add(0).write_volatile(iova as u32);
        prdt.add(1).write_volatile((iova >> 32) as u32);
        prdt.add(2).write_volatile(0);
        prdt.add(3).write_volatile((byte_count - 1) & 0x3f_ffff); // DBC = bytes - 1
    }
}

/// Issue the command in slot 0 and poll for completion. `data_iova` is the
/// DMA-mapped data buffer (0 for a data-less command such as FLUSH).
fn submit_and_wait(
    port: usize,
    cmd_list: &QueueMemory,
    cmd_table: &QueueMemory,
    command: u8,
    lba: u64,
    count: u16,
    data_iova: u64,
    byte_count: u32,
) -> bool {
    let pb = port_base(port);
    write_fis(cmd_table, command, lba, count);
    if data_iova != 0 {
        write_prdt(cmd_table, data_iova, byte_count);
    }

    // Command header for slot 0: CFL + (W for disk writes) + PRDTL.
    let header = cmd_list.vaddr as *mut u32;
    let prdtl = if data_iova != 0 { CMDHDR_PRDTL } else { 0 };
    let write_bit = if command == ATA_WRITE_DMA_EXT { CMDHDR_WRITE } else { 0 };
    unsafe {
        header.add(0).write_volatile(CMDHDR_CFL | write_bit | prdtl);
        header.add(1).write_volatile(0);
        header.add(2).write_volatile(cmd_table.iova as u32);
        header.add(3).write_volatile((cmd_table.iova >> 32) as u32);
    }
    core::sync::atomic::fence(Ordering::Release);
    write_mmio32(pb + PXCI, 1);

    for _ in 0..1_000_000 {
        if read_mmio32(pb + PXCI) & 1 == 0 {
            break;
        }
        core::hint::spin_loop();
    }
    // The task-file status carries ERR in bit 0 after a failed command.
    let tfd = read_mmio32(pb + PXTFD);
    let status = (tfd & 0xff) as u8;
    if status & PXTFD_ERR != 0 {
        return false;
    }
    // Guard against a still-busy/error device after the slot cleared.
    if status & (PXTFD_BSY | PXTFD_DRQ) != 0 {
        return false;
    }
    true
}

fn identify(port: usize, cmd_list: &QueueMemory, cmd_table: &QueueMemory, buf: &QueueMemory) -> Option<(u32, u32)> {
    if !submit_and_wait(port, cmd_list, cmd_table, ATA_IDENTIFY, 0, 1, buf.iova, 512) {
        return None;
    }
    let words = buf.vaddr as *const u16;
    // Prefer the logical-sector-size words (117/118) when valid, else 512.
    let lss = unsafe { words.add(117).read_volatile() };
    let block_size = if lss & (1 << 12) != 0 {
        (lss as u32) | ((unsafe { words.add(118).read_volatile() } as u32) << 16)
    } else {
        512
    };
    // LBA48 capacity (words 100-103).
    let total = unsafe {
        (words.add(100).read_volatile() as u64)
            | ((words.add(101).read_volatile() as u64) << 16)
            | ((words.add(102).read_volatile() as u64) << 32)
            | ((words.add(103).read_volatile() as u64) << 48)
    };
    if total == 0 {
        // Fall back to LBA28 (words 60-61).
        let lba28 = unsafe {
            (words.add(60).read_volatile() as u64) | ((words.add(61).read_volatile() as u64) << 16)
        };
        return Some((block_size, lba28 as u32));
    }
    Some((block_size, total as u32))
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

    let (mmio_map_status, hba_vaddr) = catten_syscall::device_mmio_map_any(mmio_cap, true);
    if mmio_map_status != 0 {
        unsafe { thread_exit() };
    }
    HBA_BASE.store(hba_vaddr as u64, Ordering::Release);
    config::write::<u32>(0, 1);
    config::write::<u32>(4, 10);

    // Enable the HBA, then locate an implemented port with a device attached.
    write_mmio32(HBA_GHC, GHC_AE);
    let implemented = read_mmio32(HBA_PI);
    let mut port = None;
    for index in 0..32 {
        if implemented & (1 << index) == 0 {
            continue;
        }
        let pb = port_base(index);
        let ssts = read_mmio32(pb + PXSSTS);
        if ssts & 0x0f == 0x3 {
            port = Some(index);
            break;
        }
    }
    let Some(port) = port else {
        config::write::<u32>(4, 0xe2);
        unsafe { thread_exit() };
    };
    let pb = port_base(port);
    config::write::<u32>(0, 2);
    config::write::<u32>(4, 12);

    // Stop the port, allocate the DMA structures, then (re)start it.
    write_mmio32(pb + PXCMD, 0);
    let Some(cmd_list) = alloc_dma_buffer(1024) else {
        unsafe { thread_exit() };
    };
    let Some(fis) = alloc_dma_buffer(256) else {
        unsafe { thread_exit() };
    };
    let Some(cmd_table) = alloc_dma_buffer(PAGE_SIZE) else {
        unsafe { thread_exit() };
    };
    let Some(identify_buf) = alloc_dma_buffer(512) else {
        unsafe { thread_exit() };
    };

    // Command list base (1 KiB aligned) and FIS base (256 B aligned).
    write_mmio32(pb + PXCLB, cmd_list.iova as u32);
    write_mmio32(pb + PXCLB + 4, (cmd_list.iova >> 32) as u32);
    write_mmio32(pb + PXFB, fis.iova as u32);
    write_mmio32(pb + PXFB + 4, (fis.iova >> 32) as u32);
    // Clear any pending error/interrupt status.
    write_mmio32(pb + PXSERR, read_mmio32(pb + PXSERR));
    write_mmio32(pb + PXIS, read_mmio32(pb + PXIS));
    write_mmio32(pb + PXIE, 0);
    // Start the command engine and FIS receive, then wait for both to come up.
    write_mmio32(pb + PXCMD, PXCMD_FRE | PXCMD_ST);
    for _ in 0..1_000_000 {
        let cmd = read_mmio32(pb + PXCMD);
        if cmd & (PXCMD_CR | PXCMD_FR) == PXCMD_CR | PXCMD_FR {
            break;
        }
        core::hint::spin_loop();
    }
    config::write::<u32>(4, 14);

    let Some((block_size, total_blocks)) = identify(port, &cmd_list, &cmd_table, &identify_buf) else {
        config::write::<u32>(4, 0xe1);
        unsafe { thread_exit() };
    };
    if block_size == 0 || block_size > 4096 || total_blocks == 0 {
        config::write::<u32>(4, 0xe3);
        unsafe { thread_exit() };
    }
    config::write::<u32>(4, 15);
    config::write::<u32>(40, block_size);
    config::write::<u64>(32, total_blocks as u64);
    config::write::<u32>(0, 3);

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
    config::write::<u32>(0, 4);
    config::write::<u32>(20, 0x900d);

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
                    ipc_reply(message.reply, ((block_size as u64) | ((total_blocks as u64) << 32)) as i64);
                }
            }
            block::OP_READ => {
                let (lba, count) = charlotte_protocol_block::unpack_lba_count(message.arg0);
                let transfer_bytes = (count as u64).checked_mul(block_size as u64);
                let Some(transfer_bytes) = transfer_bytes else {
                    ipc_reply(message.reply, block::ERR_INVALID_RANGE);
                    continue;
                };
                if count == 0
                    || transfer_bytes == 0
                    || transfer_bytes > 0x3f_ff00
                    || message.memory == 0
                    || lba.checked_add(count as u64).is_none_or(|end| end > total_blocks as u64)
                {
                    ipc_reply(message.reply, block::ERR_INVALID_RANGE);
                    continue;
                }
                let iova = unsafe { dma_map(dma_domain, message.memory, DmaDirection::DeviceWrite) };
                if iova == 0 {
                    ipc_reply(message.reply, block::ERR_IO_ERROR);
                    continue;
                }
                let ok = submit_and_wait(
                    port,
                    &cmd_list,
                    &cmd_table,
                    ATA_READ_DMA_EXT,
                    lba,
                    count as u16,
                    iova,
                    transfer_bytes as u32,
                );
                dma_unmap(dma_domain, iova);
                ipc_reply(message.reply, if ok { block::ERR_OK } else { block::ERR_IO_ERROR });
            }
            block::OP_WRITE => {
                let (lba, count) = charlotte_protocol_block::unpack_lba_count(message.arg0);
                let transfer_bytes = (count as u64).checked_mul(block_size as u64);
                let Some(transfer_bytes) = transfer_bytes else {
                    ipc_reply(message.reply, block::ERR_INVALID_RANGE);
                    continue;
                };
                if count == 0
                    || transfer_bytes == 0
                    || transfer_bytes > 0x3f_ff00
                    || message.memory == 0
                    || lba.checked_add(count as u64).is_none_or(|end| end > total_blocks as u64)
                {
                    ipc_reply(message.reply, block::ERR_INVALID_RANGE);
                    continue;
                }
                let iova = unsafe { dma_map(dma_domain, message.memory, DmaDirection::DeviceRead) };
                if iova == 0 {
                    ipc_reply(message.reply, block::ERR_IO_ERROR);
                    continue;
                }
                let ok = submit_and_wait(
                    port,
                    &cmd_list,
                    &cmd_table,
                    ATA_WRITE_DMA_EXT,
                    lba,
                    count as u16,
                    iova,
                    transfer_bytes as u32,
                );
                dma_unmap(dma_domain, iova);
                ipc_reply(message.reply, if ok { block::ERR_OK } else { block::ERR_IO_ERROR });
            }
            block::OP_FLUSH => {
                let ok = submit_and_wait(port, &cmd_list, &cmd_table, ATA_FLUSH_CACHE, 0, 0, 0, 0);
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
