//! GICv3 Locality-specific Peripheral Interrupt (LPI) support.
//!
//! LPIs (INTID >= 8192) are the interrupt type delivered by the ITS. Their
//! configuration lives in a per-system property table (one byte per LPI:
//! enable bit + priority) that every redistributor points at via
//! `GICR_PROPBASER`; pending state is recorded in `GICR_PENDBASER` and cleared
//! automatically on `EOI`. Enabling LPI delivery requires `GICR_CTLR.LPIEnable`
//! on each redistributor.

use spin::LazyLock;

use crate::{
    cpu::isa::{
        aarch64::memory::address::paddr::PAddr,
        interface::memory::address::PhysicalAddress,
    },
    memory::PHYSICAL_FRAME_ALLOCATOR,
};

/// The first LPI INTID. LPIs are always numbered from 8192.
pub const LPI_BASE: u32 = 8192;

// Redistributor (RD_base) register offsets for LPI control.
const GICR_CTLR: usize = 0x0000;
const GICR_CTLR_LPI_ENABLE: u32 = 1 << 0;
const GICR_PROPBASER: usize = 0x0070;
const GICR_PENDBASER: usize = 0x0078;

// QEMU's GICR_PROPBASER / GICR_PENDBASER layouts: IDBits at bits [0:5],
// PhysicalAddress at bits [51:12] (PROP) or [51:16] (PEND, 64 KiB aligned).
// IDBits must keep QEMU's pending-table scan (1 << (IDBits + 1) bits) inside
// the single allocated frame; 13 covers LPIs 8192..16383 within 4 KiB.
const GICR_PROP_IDBITS: u64 = 13;
const GICR_PROP_PHYADDR: u64 = 0x0000_ffff_ffff_f000; // bits [51:12]
const GICR_PEND_PHYADDR: u64 = 0x0000_ffff_ffff_0000; // bits [51:16]

/// LPI priority (upper nibble of the config byte). Numerically lower values are
/// higher priority; the value must sit below the `ICC_PMR_EL1` threshold
/// (`0xff`) to pass the priority filter. QEMU reads the enable bit as bit 7
/// (`LPI_CTE_ENABLED`) and the priority as `byte & 0xfc`, so with the
/// `(PRIO << 4) | 1` encoding `0x09` produces byte `0x91`: enabled, effective
/// priority `0x90` — strictly above the timer PPI's `DEFAULT_PRIORITY` (0xa0)
/// so a pending LPI wins the cached-hppi tie against the continuously
/// re-pended per-LP timer.
const LPI_PRIORITY: u8 = 0x09;

/// Allocate a zeroed frame that is `alignment`-aligned. The physical frame
/// allocator hands out page frames; this skips any that are not aligned,
/// leaking the skipped ones (acceptable for one-time LPI table setup).
fn alloc_aligned_frame(alignment: usize) -> Option<PAddr> {
    for _ in 0..(alignment / crate::cpu::isa::aarch64::memory::paging::PAGE_SIZE) {
        let frame = PHYSICAL_FRAME_ALLOCATOR.lock().allocate_frame().ok()?;
        if u64::from(frame) % alignment as u64 == 0 {
            unsafe {
                core::ptr::write_bytes(
                    frame.into_hhdm_mut::<u8>(),
                    0,
                    alignment.min(crate::cpu::isa::aarch64::memory::paging::PAGE_SIZE),
                );
            }
            return Some(frame);
        }
    }
    None
}

/// Shared property table: one byte per LPI, indexed by `intid - LPI_BASE`.
static PROP_TABLE: LazyLock<Option<PAddr>> = LazyLock::new(|| {
    let frame = PHYSICAL_FRAME_ALLOCATOR.lock().allocate_frame().ok()?;
    unsafe {
        core::ptr::write_bytes(
            frame.into_hhdm_mut::<u8>(),
            0,
            crate::cpu::isa::aarch64::memory::paging::PAGE_SIZE,
        )
    };
    Some(frame)
});

/// Shared pending table: one bit per LPI, 64 KiB-aligned per QEMU's
/// GICR_PENDBASER. The GIC writes pending state here and clears it on EOI.
static PEND_TABLE: LazyLock<Option<PAddr>> = LazyLock::new(|| alloc_aligned_frame(0x1_0000));

/// Whether `intid` is an LPI.
pub fn is_lpi(intid: u32) -> bool {
    intid >= LPI_BASE
}

/// Point this core's redistributor at the shared property/pending tables and
/// enable LPI delivery. Idempotent; safe to run from each core as it comes
/// online.
pub fn configure_lpis() {
    let (Some(prop), Some(pend)) = (*PROP_TABLE, *PEND_TABLE) else {
        return;
    };
    let base = super::gicr_rd_base();
    unsafe {
        super::mmio_write64(
            base,
            GICR_PROPBASER,
            (u64::from(prop) & GICR_PROP_PHYADDR) | GICR_PROP_IDBITS,
        );
        super::mmio_write64(base, GICR_PENDBASER, u64::from(pend) & GICR_PEND_PHYADDR);
        // Enabling LPI delivery makes the redistributor walk its pending
        // table; wait for that write to complete (RWP) as `enable_private_int`
        // does for the PPI configuration, so the redistributor is quiescent
        // before any SPI is routed.
        let ctlr = super::mmio_read32(base, GICR_CTLR);
        super::mmio_write32(base, GICR_CTLR, ctlr | GICR_CTLR_LPI_ENABLE);
        while super::mmio_read32(base, GICR_CTLR) & super::GICR_CTLR_RWP != 0 {
            core::hint::spin_loop();
        }
    }
}

/// Enable or disable `intid` in the redistributor property table. The caller
/// must follow with an ITS `INV`/`INVALL` so the change is observed.
pub fn set_lpi_enabled(intid: u32, enabled: bool) {
    let Some(prop) = *PROP_TABLE else {
        return;
    };
    if !is_lpi(intid) {
        return;
    }
    let ptr = unsafe { prop.into_hhdm_mut::<u8>().add((intid - LPI_BASE) as usize) };
    unsafe {
        let byte = (LPI_PRIORITY << 4) | enabled as u8;
        core::ptr::write_volatile(ptr, byte);
        core::arch::asm!("dsb ishst", options(nomem, nostack, preserves_flags));
    }
}
