//! NVMe block device driver for CharlotteOS.
//!
//! Implements an NVM Express 1.4 driver as a userspace EL0 service. Discovers
//! the controller via delegated BAR0 MMIO + interrupt grants (from the PCI
//! topology scanner, or hardcoded for QEMU virt in test mode), initialises the
//! admin and I/O queues, identifies the first namespace, and serves the block
//! device protocol (`charlotte-protocol-block` v1).
//!
//! ## Initialisation sequence
//!
//! 1. Reset controller: disable CC.EN, wait for CSTS.RDY=0.
//! 2. Configure admin queues: allocate contiguous physical memory for ASQ/ACQ (32 entries each),
//!    write AQA/ASQ/ACQ registers, enable controller.
//! 3. Identify controller (CNS=1) → read CAP, VS, etc.
//! 4. Identify namespace 1 (CNS=0) → read block size (LBAF) and total blocks (NSZE).
//! 5. Create I/O Completion Queue (qid=1): allocate memory, submit Create I/O CQ admin command.
//! 6. Create I/O Submission Queue (qid=1): allocate memory, submit Create I/O SQ admin command.
//! 7. Register with the name service as `"blk0"`, enter the unified shard-wait loop.  READ/WRITE
//!    commands submit NVM Read/Write to the I/O SQ and retain reply tokens; completions arrive via
//!    the bound interrupt → CQ wake.
//!
//! ## Completion model
//!
//! The bring-up path uses bounded CQ polling because QEMU's NVMe PCI function
//! reports no legacy interrupt line and CharlotteOS does not yet program MSI or
//! MSI-X. `cq_wait_timeout` provides the required VM exit without busy-spinning
//! and also wakes immediately for endpoint work. The delegated IRQ is still
//! bound and acknowledged when present, so enabling MSI/MSI-X later does not
//! require changing the service loop.
#![no_std]
#![no_main]
extern crate alloc;

use core::sync::atomic::{
    AtomicU32,
    AtomicUsize,
    Ordering,
};

catten_rt::entry!(main);

// Simple bump allocator for queue virtual addresses
static NEXT_VADDR: AtomicUsize = AtomicUsize::new(0x0000_0000_0050_0000);

fn alloc_vaddr(pages: usize) -> usize {
    NEXT_VADDR.fetch_add(pages * PAGE_SIZE, Ordering::Relaxed)
}

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    block,
    ns,
};
use catten_syscall::{
    ipc_status,
    *,
};

const NVME_VADDR: usize = 0x0000_0000_0041_0000;
const ADMIN_QUEUE_SIZE: u32 = 32;
const IO_QUEUE_SIZE: u32 = 64;
const MAX_TRANSFER_BYTES: u64 = PAGE_SIZE as u64;
const PAGE_SIZE: usize = 4096;

fn spin_reply(call: u64) -> (i64, u64) {
    for _ in 0..500_000 {
        let (status, result, cap) = ipc_reply_poll(call);
        if status == 0 {
            ipc_close(call);
            return (result as i64, cap);
        }
        core::hint::spin_loop();
    }
    ipc_close(call);
    (-1, 0)
}

// ---------------------------------------------------------------------------
// NVMe controller register offsets (relative to BAR0)
// ---------------------------------------------------------------------------
mod reg {
    pub const _CAP: usize = 0x0000;
    pub const _VS: usize = 0x0008;
    pub const _INTMS: usize = 0x000c;
    pub const _INTMC: usize = 0x0010;
    pub const CC: usize = 0x0014;
    pub const CSTS: usize = 0x001c;
    pub const AQA: usize = 0x0024;
    pub const ASQ: usize = 0x0028;
    pub const ACQ: usize = 0x0030;
}

// CC register bits
const CC_EN: u32 = 1 << 0;
const CC_IOSQES: u32 = 6 << 16;
const CC_IOCQES: u32 = 4 << 20;

// CSTS register bits
const CSTS_RDY: u32 = 1 << 0;

// Admin opcodes
const _ADMIN_DELETE_IO_SQ: u8 = 0x00;
const ADMIN_CREATE_IO_SQ: u8 = 0x01;
const _ADMIN_DELETE_IO_CQ: u8 = 0x04;
const ADMIN_CREATE_IO_CQ: u8 = 0x05;
const ADMIN_IDENTIFY: u8 = 0x06;
const ADMIN_GET_FEATURES: u8 = 0x0a;
const ADMIN_SET_FEATURES: u8 = 0x09;

// NVM command opcodes
const NVM_FLUSH: u8 = 0x00;
const NVM_WRITE: u8 = 0x01;
const NVM_READ: u8 = 0x02;

// Phase tag
const _CQE_PHASE: u16 = 1 << 0;
const _CQE_SF_MASK: u16 = 0xff << 1;

// ---------------------------------------------------------------------------
// Doorbell offsets from BAR0 base
// ---------------------------------------------------------------------------
static mut DOORBELL_STRIDE: usize = 4;

fn sq0_tdbl() -> usize {
    0x1000
}
fn cq0_hdbl() -> usize {
    0x1000 + unsafe { DOORBELL_STRIDE }
}
fn sq1_tdbl() -> usize {
    0x1000 + 2 * unsafe { DOORBELL_STRIDE }
}
fn cq1_hdbl() -> usize {
    0x1000 + 3 * unsafe { DOORBELL_STRIDE }
}

// ---------------------------------------------------------------------------
// MMIO helpers
// ---------------------------------------------------------------------------
unsafe fn read32(offset: usize) -> u32 {
    unsafe { core::ptr::read_volatile((NVME_VADDR + offset) as *const u32) }
}

unsafe fn write32(offset: usize, val: u32) {
    unsafe { core::ptr::write_volatile((NVME_VADDR + offset) as *mut u32, val) }
}

unsafe fn write64(offset: usize, val: u64) {
    unsafe { core::ptr::write_volatile((NVME_VADDR + offset) as *mut u64, val) }
}

/// Write a doorbell value (low 16 bits written to 32-bit doorbell register).
unsafe fn doorbell_write(offset: usize, val: u16) {
    unsafe { write32(offset, val as u32) }
}

// ---------------------------------------------------------------------------
// Memory allocation helpers
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct QueueMemory {
    cap: u64,
    phys: u64,
    vaddr: usize,
    pages: usize,
}

/// Allocate physically contiguous, page-aligned memory for a queue.
fn alloc_queue_memory(entries: usize, entry_size: usize) -> Option<QueueMemory> {
    let total_bytes = entries * entry_size;
    let pages = (total_bytes + PAGE_SIZE - 1) / PAGE_SIZE;
    let cap = memory_alloc(pages);
    if cap == 0 {
        return None;
    }
    let vaddr = alloc_vaddr(pages);
    if memory_map(cap, vaddr, true) != 0 {
        memory_close(cap);
        return None;
    }
    let phys = memory_get_phys(cap);
    if phys == 0 {
        memory_unmap(cap);
        memory_close(cap);
        return None;
    }
    Some(QueueMemory {
        cap,
        phys,
        vaddr,
        pages,
    })
}

// ---------------------------------------------------------------------------
// Admin queue: submission + completion entry formats
// ---------------------------------------------------------------------------

/// Build a 64-byte admin SQE at `base + 64*slot`.
///
/// SQE layout (64 bytes, 8 × u64):
///   [0]: DW0 (u32 lo) | NSID (u32 hi)
///   [1]: Reserved (u64)
///   [2]: MPTR (u64)
///   [3]: PRP1 (u64)
///   [4]: PRP2 (u64)
///   [5]: CDW10 (u32 lo) | CDW11 (u32 hi)
///   [6]: CDW12 (u32 lo) | CDW13 (u32 hi)
///   [7]: CDW14 (u32 lo) | CDW15 (u32 hi)
unsafe fn admin_sqe(
    base: usize,
    slot: u32,
    opcode: u8,
    nsid: u32,
    cdw10: u32,
    cdw11: u32,
    prp1: u64,
) {
    let off = base + (slot as usize) * 64;
    let dw0: u32 = ((slot & 0xffff) << 16) | (opcode as u32 & 0xff);
    unsafe {
        let p = off as *mut u64;
        p.write_volatile((nsid as u64) << 32 | dw0 as u64);
        p.add(1).write_volatile(0);
        p.add(2).write_volatile(0);
        p.add(3).write_volatile(prp1);
        p.add(4).write_volatile(0);
        p.add(5).write_volatile((cdw11 as u64) << 32 | cdw10 as u64);
        p.add(6).write_volatile(0);
        p.add(7).write_volatile(0);
    }
}

/// Read a 16-byte CQE at `base + 16*slot`, returns (phase_match, sf, cid).
unsafe fn read_cqe(base: usize, slot: u32, expected_phase: u8) -> (bool, u16, u16) {
    let off = base + (slot as usize) * 16;
    unsafe {
        let p = off as *const u32;
        let _dw0 = p.read_volatile();
        let _rsvd = p.add(1).read_volatile();
        let _dw2 = p.add(2).read_volatile();
        let dw3 = p.add(3).read_volatile();
        let cid = (dw3 & 0xffff) as u16;
        let status_field = ((dw3 >> 16) & 0xfffe) as u16;
        let phase = ((dw3 >> 16) & 0x1) as u8;
        (phase == expected_phase, status_field, cid)
    }
}

// ---------------------------------------------------------------------------
// I/O queue: NVM command helpers
// ---------------------------------------------------------------------------

/// Build a 64-byte NVM SQE (Read or Write) at `base + 64*slot`.
unsafe fn nvm_sqe(
    base: usize,
    slot: u32,
    opcode: u8,
    nsid: u32,
    start_lba: u64,
    nblocks: u16,
    prp1: u64,
) {
    let off = base + (slot as usize) * 64;
    let dw0: u32 = ((slot & 0xffff) << 16) | (opcode as u32 & 0xff);
    unsafe {
        let p = off as *mut u64;
        p.write_volatile((nsid as u64) << 32 | dw0 as u64);
        p.add(1).write_volatile(0);
        p.add(2).write_volatile(0);
        p.add(3).write_volatile(prp1);
        p.add(4).write_volatile(0);
        p.add(5).write_volatile(start_lba);
        p.add(6).write_volatile((nblocks.saturating_sub(1) as u64) & 0xffff);
        p.add(7).write_volatile(0);
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static BLOCK_SIZE: AtomicU32 = AtomicU32::new(0);
static TOTAL_BLOCKS: AtomicU32 = AtomicU32::new(0);

/// Outstanding I/O operation: a retained reply token waiting for a completion.
const MAX_PENDING: usize = 64;
static PENDING_REPLIES: [AtomicU32; MAX_PENDING] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];

fn store_pending(slot: u32, reply: u64) {
    PENDING_REPLIES[(slot as usize) % MAX_PENDING].store(reply as u32, Ordering::Release);
}

fn take_pending(slot: u32) -> u64 {
    let prev = PENDING_REPLIES[(slot as usize) % MAX_PENDING].swap(0, Ordering::AcqRel);
    prev as u64
}

// ---------------------------------------------------------------------------
// Admin command submit + poll-for-completion
// ---------------------------------------------------------------------------

struct AdminQueues {
    sq_base: usize,
    cq_base: usize,
    sq_tail: u32,
    cq_head: u32,
    cq_phase: u8,
}

unsafe fn admin_submit_and_wait(
    aq: &mut AdminQueues,
    opcode: u8,
    nsid: u32,
    cdw10: u32,
    cdw11: u32,
    prp1: u64,
) -> u32 {
    let slot = aq.sq_tail;
    unsafe {
        admin_sqe(aq.sq_base, slot, opcode, nsid, cdw10, cdw11, prp1);
    }
    // Read back the SQE we just wrote: DW0 (opcode+CID), DW3 (PRP1), DW5 (CDW10-11)
    let sqe_off = aq.sq_base + (slot as usize) * 64;
    let rdw0 = unsafe { core::ptr::read_volatile(sqe_off as *const u32) };
    let rdw3 = unsafe { core::ptr::read_volatile((sqe_off + 24) as *const u64) };
    let rdw5_lo = unsafe { core::ptr::read_volatile((sqe_off + 40) as *const u32) };
    let rdw5_hi = unsafe { core::ptr::read_volatile((sqe_off + 44) as *const u32) };
    config::write::<u32>(52, rdw0);
    config::write::<u64>(56, rdw3);
    config::write::<u32>(64, rdw5_lo);
    config::write::<u32>(68, rdw5_hi);
    aq.sq_tail = (slot + 1) % ADMIN_QUEUE_SIZE;
    // Release barrier: ensure SQE stores are visible to DMA before doorbell
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    unsafe {
        doorbell_write(sq0_tdbl(), aq.sq_tail as u16);
    }

    loop {
        unsafe {
            let (phase_match, sf, _cid) = read_cqe(aq.cq_base, aq.cq_head, aq.cq_phase);
            if phase_match {
                let raw_dw3 = core::ptr::read_volatile(
                    (aq.cq_base + (aq.cq_head as usize) * 16 + 12) as *const u32,
                );
                config::write::<u32>(44, raw_dw3);
                aq.cq_head = (aq.cq_head + 1) % ADMIN_QUEUE_SIZE;
                if aq.cq_head == 0 {
                    aq.cq_phase ^= 1;
                }
                doorbell_write(cq0_hdbl(), aq.cq_head as u16);
                return sf as u32;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Controller initialisation
// ---------------------------------------------------------------------------

unsafe fn nvme_init() -> Option<(usize, usize, u64, u32, u32)> {
    config::write::<u32>(4, 10); // entering nvme_init
    // 1. Reset controller: disable, wait for RDY=0 with a timeout
    unsafe {
        write32(reg::CC, 0);
        for _ in 0..100_000 {
            if read32(reg::CSTS) & CSTS_RDY == 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }
    config::write::<u32>(4, 11); // controller reset

    // Read CAP with two 32-bit reads, set doorbell stride from DSTRD (bits 35:32)
    let cap_lo = unsafe { read32(0x0000) };
    let cap_hi = unsafe { read32(0x0004) };
    let dstrd = ((cap_hi as u64) & 0xf) as usize;
    let dbl_stride = 4usize << dstrd;
    config::write::<u32>(20, cap_lo);
    config::write::<u32>(24, cap_hi);
    config::write::<u32>(28, dbl_stride as u32);
    unsafe {
        DOORBELL_STRIDE = dbl_stride;
    }

    // 2. Allocate admin queues
    let asq_mem = alloc_queue_memory(ADMIN_QUEUE_SIZE as usize, 64)?;
    config::write::<u32>(4, 12); // ASQ allocated
    let acq_mem = alloc_queue_memory(ADMIN_QUEUE_SIZE as usize, 16)?;
    config::write::<u32>(4, 13); // ACQ allocated

    unsafe {
        write32(reg::AQA, (ADMIN_QUEUE_SIZE - 1) | ((ADMIN_QUEUE_SIZE - 1) << 16));
        write64(reg::ASQ, asq_mem.phys);
        write64(reg::ACQ, acq_mem.phys);
        write32(reg::CC, CC_EN | CC_IOSQES | CC_IOCQES);
        while read32(reg::CSTS) & CSTS_RDY == 0 {
            core::hint::spin_loop();
        }
    }
    config::write::<u32>(4, 14); // controller enabled

    let mut aq = AdminQueues {
        sq_base: asq_mem.vaddr,
        cq_base: acq_mem.vaddr,
        sq_tail: 0,
        cq_head: 0,
        cq_phase: 1,
    };

    // Zero the admin CQ entries so stale phase bits don't look like completions.
    unsafe {
        core::ptr::write_bytes(acq_mem.vaddr as *mut u8, 0, ADMIN_QUEUE_SIZE as usize * 16);
    }

    // 3. Identify controller (CNS=1) — allocate memory for the 4096-byte data structure
    let id_ctrl_mem = alloc_queue_memory(1, PAGE_SIZE)?;
    config::write::<u32>(4, 19); // id-ctrl buffer allocated
    let sf = unsafe { admin_submit_and_wait(&mut aq, ADMIN_IDENTIFY, 0, 1, 0, id_ctrl_mem.phys) };
    config::write::<u32>(4, 20); // admin_submit_and_wait returned
    if sf != 0 {
        return None;
    }
    config::write::<u32>(4, 15); // identify-ctrl ok
    let oacs: u16 = unsafe { core::ptr::read_volatile((id_ctrl_mem.vaddr + 256) as *const u16) };
    config::write::<u32>(76, oacs as u32);

    let oacs: u16 = unsafe { core::ptr::read_volatile((id_ctrl_mem.vaddr + 256) as *const u16) };
    config::write::<u32>(76, oacs as u32);

    // Verify admin queue still works: Get Features (Arbitration, FID=1)
    let test_mem = alloc_queue_memory(1, PAGE_SIZE)?;
    let test_sf =
        unsafe { admin_submit_and_wait(&mut aq, ADMIN_GET_FEATURES, 0, 1, 0, test_mem.phys) };
    config::write::<u32>(48, test_sf);

    // Set Features: Number of Queues (FID=0x07). CDW11 = (NCQR << 16) | NSQR.
    // Request 1 I/O CQ and 1 I/O SQ.
    let nq_cdw10: u32 = 7; // FID=7
    let nq_cdw11: u32 = (1u32 << 16) | 1; // NCQR=1, NSQR=1
    let nq_sf =
        unsafe { admin_submit_and_wait(&mut aq, ADMIN_SET_FEATURES, 0, nq_cdw10, nq_cdw11, 0) };
    config::write::<u32>(72, nq_sf);

    // 4. Identify namespace 1 (CNS=0)
    let id_ns_mem = alloc_queue_memory(1, PAGE_SIZE)?;
    let sf = unsafe { admin_submit_and_wait(&mut aq, ADMIN_IDENTIFY, 1, 0, 0, id_ns_mem.phys) };
    if sf != 0 {
        return None;
    }
    config::write::<u32>(4, 16); // identify-ns ok

    let nsze = unsafe { core::ptr::read_volatile(id_ns_mem.vaddr as *const u64) };
    let flbas = unsafe { core::ptr::read_volatile((id_ns_mem.vaddr + 26) as *const u8) };
    let lbaf_idx = flbas & 0xf;
    let lbaf_off = 128 + (lbaf_idx as usize) * 4;
    let lbaf_raw = unsafe { core::ptr::read_volatile((id_ns_mem.vaddr + lbaf_off) as *const u32) };
    // LBAF: metadata size is bits 15:0, LBA data-size exponent (LBADS)
    // is bits 23:16, and relative performance is bits 25:24.
    let lbads = (lbaf_raw >> 16) & 0xff;
    if !(9..=12).contains(&lbads) {
        return None;
    }
    let lbs = 1u32 << lbads;
    config::write::<u64>(32, nsze);
    config::write::<u32>(40, lbs);

    // 5. Create I/O CQ first. Zero CQ memory before queue creation.
    let io_cq_mem = alloc_queue_memory(1, PAGE_SIZE)?;
    unsafe {
        core::ptr::write_bytes(io_cq_mem.vaddr as *mut u8, 0, PAGE_SIZE);
    }
    let cq_cdw10: u32 = ((IO_QUEUE_SIZE - 1) << 16) | 1;
    let cq_cdw11: u32 = 1; // PC=1, IEN=0 until PCI MSI/MSI-X is configured
    let cq_sf = unsafe {
        admin_submit_and_wait(&mut aq, ADMIN_CREATE_IO_CQ, 0, cq_cdw10, cq_cdw11, io_cq_mem.phys)
    };
    config::write::<u32>(12, cq_sf);
    if cq_sf != 0 {
        return None;
    }
    config::write::<u32>(4, 17);
    // 6. Create I/O SQ
    let io_sq_mem = alloc_queue_memory(1, PAGE_SIZE)?;
    unsafe {
        core::ptr::write_bytes(io_sq_mem.vaddr as *mut u8, 0, PAGE_SIZE);
    }
    let sq_cdw10: u32 = ((IO_QUEUE_SIZE - 1) << 16) | 1;
    let sq_cdw11: u32 = (1u32 << 16) | 1; // CQID=1, PC=1
    let sq_sf = unsafe {
        admin_submit_and_wait(&mut aq, ADMIN_CREATE_IO_SQ, 0, sq_cdw10, sq_cdw11, io_sq_mem.phys)
    };
    config::write::<u32>(16, sq_sf);
    if sq_sf != 0 {
        return None;
    }
    config::write::<u32>(4, 18);
    Some((io_sq_mem.vaddr, io_cq_mem.vaddr, nsze, lbs, 0))
}

// ---------------------------------------------------------------------------
// I/O submission
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct IoState {
    sq_vaddr: usize,
    cq_vaddr: usize,
    sq_tail: u32,
    cq_head: u32,
    cq_phase: u8,
    nsze: u64,
    lbs: u32,
    cmd_slot: u32,
    outstanding: u32,
}

impl IoState {
    fn submit_io(&mut self, opcode: u8, start_lba: u64, nblocks: u16, prp1: u64) -> Option<u32> {
        // A circular SQ must always leave one entry unused so full and empty
        // remain distinguishable.
        if self.outstanding >= IO_QUEUE_SIZE - 1 {
            return None;
        }
        let slot = self.sq_tail;
        unsafe {
            nvm_sqe(self.sq_vaddr, slot, opcode, 1, start_lba, nblocks, prp1);
        }
        self.sq_tail = (slot + 1) % IO_QUEUE_SIZE;
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        unsafe {
            doorbell_write(sq1_tdbl(), self.sq_tail as u16);
        }
        self.cmd_slot = slot;
        self.outstanding += 1;
        Some(slot)
    }

    fn poll_completions(&mut self) -> Option<(u16, u16)> {
        unsafe {
            let cqe_ptr = (self.cq_vaddr + (self.cq_head as usize) * 16) as *const u32;
            let dw3 = core::ptr::read_volatile(cqe_ptr.add(3));
            let phase = ((dw3 >> 16) & 1) as u8;
            if phase != self.cq_phase {
                return None;
            }
            core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
            let cid = (dw3 & 0xffff) as u16;
            let status = ((dw3 >> 17) & 0x7fff) as u16;
            self.cq_head = (self.cq_head + 1) % IO_QUEUE_SIZE;
            if self.cq_head == 0 {
                self.cq_phase ^= 1;
            }
            self.outstanding = self.outstanding.saturating_sub(1);
            core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
            doorbell_write(cq1_hdbl(), self.cq_head as u16);
            Some((cid, status))
        }
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main(ctx: Context) -> ! {
    let ns_connection = match ctx.bootstrap_cap() {
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

    if device_mmio_map(mmio_cap, NVME_VADDR, true) != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 1);

    let (io_sq_vaddr, io_cq_vaddr, nsze, lbs, _) = match unsafe { nvme_init() } {
        Some(v) => v,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(0, 2);

    let total_blocks = nsze as u32;
    BLOCK_SIZE.store(lbs, Ordering::Release);
    TOTAL_BLOCKS.store(total_blocks, Ordering::Release);

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
    device_irq_bind_cq(irq_cap, 0);
    config::write::<u32>(0, 3); // registered, serving
    config::write::<u32>(20, 0x900d); // sentinel for verifier

    let mut io = IoState {
        sq_vaddr: io_sq_vaddr,
        cq_vaddr: io_cq_vaddr,
        sq_tail: 0,
        cq_head: 0,
        cq_phase: 1,
        nsze,
        lbs,
        cmd_slot: 0,
        outstanding: 0,
    };

    let mut irq_count: u32 = 0;
    let blk = lbs;
    let tot = total_blocks;

    loop {
        // Poll I/O completions before cq_wait (catches completions from
        // commands submitted in the previous iteration)
        while let Some((cid, status)) = io.poll_completions() {
            let reply = take_pending(cid as u32);
            if reply != 0 {
                ipc_reply(
                    reply,
                    if status == 0 {
                        block::ERR_OK
                    } else {
                        block::ERR_IO_ERROR
                    },
                );
            }
        }

        // A bounded wait causes the VM exit TCG needs to process DMA, wakes
        // immediately for endpoint/IRQ readiness, and polls the CQ at 1 ms
        // intervals when the PCI function has no configured interrupt route.
        cq_wait_timeout(1, 1, 0);

        let (status, consumed) = device_irq_ack(irq_cap);
        if status == 0 && consumed > 0 {
            irq_count = irq_count.saturating_add(consumed as u32);
        }
        while let Some((cid, status)) = io.poll_completions() {
            let reply = take_pending(cid as u32);
            if reply != 0 {
                ipc_reply(
                    reply,
                    if status == 0 {
                        block::ERR_OK
                    } else {
                        block::ERR_IO_ERROR
                    },
                );
            }
        }

        loop {
            let message = ipc_recv(endpoint);
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
                block::OP_INFO => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, ((blk as u64) | ((tot as u64) << 32)) as i64);
                    }
                }
                block::OP_READ => {
                    if message.reply != 0 {
                        if io.sq_vaddr == 0 {
                            ipc_reply(message.reply, block::ERR_IO_ERROR);
                            continue;
                        }
                        let (lba, count) = {
                            let (l, c) = charlotte_protocol_block::unpack_lba_count(message.arg0);
                            (l, c as u32)
                        };
                        let transfer_bytes = (count as u64).checked_mul(lbs as u64);
                        if count == 0 || lba.checked_add(count as u64).is_none_or(|end| end > nsze)
                        {
                            ipc_reply(message.reply, block::ERR_INVALID_RANGE);
                            continue;
                        }
                        if transfer_bytes.is_none_or(|bytes| bytes > MAX_TRANSFER_BYTES) {
                            ipc_reply(message.reply, block::ERR_UNALIGNED);
                            continue;
                        }
                        let mem_cap = message.memory;
                        if mem_cap == 0 {
                            ipc_reply(message.reply, block::ERR_IO_ERROR);
                            continue;
                        }
                        let phys = memory_get_phys(mem_cap);
                        if phys == 0 {
                            ipc_reply(message.reply, block::ERR_IO_ERROR);
                            continue;
                        }
                        match io.submit_io(NVM_READ, lba, count as u16, phys) {
                            Some(slot) => store_pending(slot, message.reply),
                            None => {
                                ipc_reply(message.reply, block::ERR_IO_ERROR);
                            }
                        }
                    }
                }
                block::OP_WRITE => {
                    if message.reply != 0 {
                        if io.sq_vaddr == 0 {
                            ipc_reply(message.reply, block::ERR_IO_ERROR);
                            continue;
                        }
                        let (lba, count) = {
                            let (l, c) = charlotte_protocol_block::unpack_lba_count(message.arg0);
                            (l, c as u32)
                        };
                        let transfer_bytes = (count as u64).checked_mul(lbs as u64);
                        if count == 0 || lba.checked_add(count as u64).is_none_or(|end| end > nsze)
                        {
                            ipc_reply(message.reply, block::ERR_INVALID_RANGE);
                            continue;
                        }
                        if transfer_bytes.is_none_or(|bytes| bytes > MAX_TRANSFER_BYTES) {
                            ipc_reply(message.reply, block::ERR_UNALIGNED);
                            continue;
                        }
                        let mem_cap = message.memory;
                        if mem_cap == 0 {
                            ipc_reply(message.reply, block::ERR_IO_ERROR);
                            continue;
                        }
                        let phys = memory_get_phys(mem_cap);
                        if phys == 0 {
                            ipc_reply(message.reply, block::ERR_IO_ERROR);
                            continue;
                        }
                        match io.submit_io(NVM_WRITE, lba, count as u16, phys) {
                            Some(slot) => store_pending(slot, message.reply),
                            None => {
                                ipc_reply(message.reply, block::ERR_IO_ERROR);
                            }
                        }
                    }
                }
                block::OP_FLUSH => {
                    if message.reply != 0 {
                        match io.submit_io(NVM_FLUSH, 0, 0, 0) {
                            Some(slot) => store_pending(slot, message.reply),
                            None => {
                                ipc_reply(message.reply, block::ERR_IO_ERROR);
                            }
                        }
                    }
                }
                block::OP_TRIM => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, block::ERR_IO_ERROR);
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
