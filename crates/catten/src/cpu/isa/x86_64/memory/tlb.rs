use core::arch::asm;

use crate::{
    cpu::isa::memory::paging::PAGE_SIZE,
    memory::{
        AddressSpaceId,
        VAddr,
    },
};

/// CharlotteOS does not yet enable CR4.PCIDE or assign x86 PCIDs. Reloading
/// CR3 therefore invalidates all non-global translations for the address
/// space currently active on this LP. That is sufficient for a shootdown:
/// if `asid` is active it is flushed now, and if it is inactive its next
/// context switch reloads CR3 and flushes it before use.
pub fn flush_current_non_global() {
    unsafe {
        asm!(
            "mov {cr3}, cr3",
            "mov cr3, {cr3}",
            cr3 = out(reg) _,
            options(nostack, preserves_flags),
        );
    }
}

/// Invalidate a single translation locally (the currently active CR3) with
/// `invlpg`. Safe in any context; the cross-LP part is the synchronous shootdown
/// in [`send_sync_shootdown`](crate::cpu::isa::x86_64::interrupts::fixed::ipis::send_sync_shootdown).
#[inline]
fn invlpg(page: usize) {
    unsafe {
        asm!(
            "invlpg [{page}]",
            page = in(reg) page,
            options(nostack, preserves_flags),
        );
    }
}

fn invlpg_range(base: VAddr, num_pages: usize) {
    let raw_base = <VAddr as Into<usize>>::into(base);
    for page in (raw_base..raw_base + num_pages * PAGE_SIZE).step_by(PAGE_SIZE) {
        invlpg(page);
    }
}

/// Invalidate a range of user page translations across every LP. Locally
/// invalidates each translation and then performs a synchronous cross-LP
/// shootdown (a CR3 reload on every other LP).
pub fn inval_range_user(_asid: AddressSpaceId, base: VAddr, size: usize) {
    invlpg_range(base, size);
    super::super::interrupts::fixed::ipis::send_sync_shootdown();
}

/// Invalidate all translations of an address space across every LP. Without
/// PCID, a CR3 reload on every LP (locally and via the synchronous shootdown)
/// flushes all non-global entries, so this is deliberately ASID-agnostic.
pub fn inval_asid(_asid: AddressSpaceId) {
    super::super::interrupts::fixed::ipis::send_sync_shootdown();
}

/// Invalidate a range of kernel page translations across every LP.
pub fn inval_range_kernel(base: VAddr, num_pages: usize) {
    invlpg_range(base, num_pages);
    super::super::interrupts::fixed::ipis::send_sync_shootdown();
}
