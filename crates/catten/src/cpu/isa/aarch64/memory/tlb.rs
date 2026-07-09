//! # AArch64 TLB Maintenance
//!
//! Unlike x86-64, where TLB shootdown across cores must be coordinated with
//! inter-processor interrupts, AArch64 provides *broadcast* TLB maintenance
//! instructions. A `TLBI ...IS` (Inner Shareable) executed on one core
//! invalidates the matching entries on every core in the inner shareable
//! domain in hardware. This matches the async-first spirit of the system: we
//! lean on the mechanism the hardware already provides rather than emulating a
//! synchronous cross-core handshake in software.
//!
//! The required barrier sequence for a break-before-make style invalidation is:
//! - `DSB ISH`  ensure prior page table stores are observed before invalidation
//! - `TLBI ...IS`  broadcast the invalidation across the inner shareable domain
//! - `DSB ISH`  wait for the broadcast invalidation to complete everywhere
//! - `ISB`  ensure subsequent instructions use the new translations
//!
//! See the ARM ARM, section D8.14 "TLB maintenance" and the description of the
//! `TLBI` instruction.

use core::arch::asm;

use crate::cpu::isa::aarch64::memory::address::vaddr::VAddr;
use crate::cpu::isa::aarch64::memory::paging::PAGE_SIZE;
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::memory::AddressSpaceId;

/// Encode a virtual address for the `TLBI ...VA*` instructions. The address is
/// shifted right by 12 (the page shift); bits [43:0] of the operand hold VA
/// bits [55:12].
#[inline(always)]
fn tlbi_va_operand(vaddr: VAddr) -> u64 {
    (<VAddr as Into<u64>>::into(vaddr) >> 12) & 0x0000_0fff_ffff_ffff
}

/// Encode an ASID for the `TLBI ...ASID*` instructions. The ASID occupies bits
/// [63:48] of the operand.
#[inline(always)]
fn tlbi_asid_operand(asid: HwAsidRaw) -> u64 {
    (asid as u64) << 48
}

type HwAsidRaw = u16;

/// Invalidate a single kernel (global) page translation across all cores.
///
/// `VAAE1IS` invalidates by VA at EL1, across all ASIDs, broadcast to the inner
/// shareable domain. Kernel mappings are global so this is the correct variant
/// for higher-half addresses.
#[inline]
pub fn inval_page(vaddr: VAddr) {
    let op = tlbi_va_operand(vaddr);
    unsafe {
        asm!(
            "dsb ishst",
            "tlbi vaae1is, {op}",
            "dsb ish",
            "isb",
            op = in(reg) op,
            options(nostack, preserves_flags)
        );
    }
}

/// Invalidate a range of kernel page translations across all cores.
pub fn inval_range_kernel(base: VAddr, num_pages: usize) {
    let raw_base = <VAddr as Into<usize>>::into(base);
    unsafe {
        asm!("dsb ishst", options(nostack, preserves_flags));
        for page in (raw_base..raw_base + num_pages * PAGE_SIZE).step_by(PAGE_SIZE) {
            let op = (page as u64 >> 12) & 0x0000_0fff_ffff_ffff;
            asm!("tlbi vaae1is, {op}", op = in(reg) op, options(nostack, preserves_flags));
        }
        asm!("dsb ish", "isb", options(nostack, preserves_flags));
    }
}

/// Invalidate a range of user page translations for a given address space
/// across all cores. Uses the ASID-qualified `VAE1IS` variant so that only the
/// target address space's entries are affected.
pub fn inval_range_user(asid: AddressSpaceId, base: VAddr, num_pages: usize) {
    let Some(hwasid) =
        SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().asid_to_hwasid(asid)
    else {
        return;
    };
    let raw_base = <VAddr as Into<usize>>::into(base);
    let asid_bits = tlbi_asid_operand(hwasid);
    unsafe {
        asm!("dsb ishst", options(nostack, preserves_flags));
        for page in (raw_base..raw_base + num_pages * PAGE_SIZE).step_by(PAGE_SIZE) {
            let op = asid_bits | ((page as u64 >> 12) & 0x0000_0fff_ffff_ffff);
            asm!("tlbi vae1is, {op}", op = in(reg) op, options(nostack, preserves_flags));
        }
        asm!("dsb ish", "isb", options(nostack, preserves_flags));
    }
}

/// Invalidate all translations belonging to an address space across all cores.
///
/// `ASIDE1IS` invalidates every entry tagged with the given ASID, broadcast to
/// the inner shareable domain.
pub fn inval_asid(asid: AddressSpaceId) {
    let Some(hwasid) =
        SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().asid_to_hwasid(asid)
    else {
        return;
    };
    let op = tlbi_asid_operand(hwasid);
    unsafe {
        asm!(
            "dsb ishst",
            "tlbi aside1is, {op}",
            "dsb ish",
            "isb",
            op = in(reg) op,
            options(nostack, preserves_flags)
        );
    }
}
