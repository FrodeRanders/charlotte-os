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
fn flush_current_non_global() {
    unsafe {
        asm!(
            "mov {cr3}, cr3",
            "mov cr3, {cr3}",
            cr3 = out(reg) _,
            options(nostack, preserves_flags),
        );
    }
}

pub fn inval_range_user(_asid: AddressSpaceId, _base: VAddr, _size: usize) {
    flush_current_non_global();
}

pub fn inval_asid(_asid: AddressSpaceId) {
    flush_current_non_global();
}

pub fn inval_range_kernel(base: VAddr, num_pages: usize) {
    let raw_base = <VAddr as Into<usize>>::into(base);
    let len_bytes = num_pages * PAGE_SIZE;
    for page in (raw_base..raw_base + len_bytes).step_by(PAGE_SIZE) {
        unsafe {
            asm!(
                "invlpg [{page}]",
                page = in(reg) page,
                options(nostack, preserves_flags),
            );
        }
    }
}
