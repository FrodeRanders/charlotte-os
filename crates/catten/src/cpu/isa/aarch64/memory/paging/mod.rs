//! # AArch64 Paging (VMSAv8-64, 4 KiB granule)

pub mod descriptor;
pub mod walker;

use core::{
    arch::asm,
    ptr::NonNull,
};

use descriptor::Descriptor;

use super::MemoryInterfaceImpl;
use crate::{
    cpu::isa::{
        aarch64::memory::address::{
            paddr::PAddr,
            vaddr::VAddr,
        },
        interface::memory::{
            address::{
                Address,
                VirtualAddress,
            },
            AddressSpaceInterface,
            MemoryInterface,
            MemoryMapping,
        },
    },
    klib::size::{
        gibibytes,
        kibibytes,
        mebibytes,
    },
};

/// Hardware Address Space Identifier. On AArch64 the ASID is held in the top
/// bits of `TTBR0_EL1`/`TTBR1_EL1` and tags TLB entries. It is 8 or 16 bits
/// wide depending on `TCR_EL1.AS`; we model the full 16-bit width and mask as
/// required.
pub type HwAsid = u16;

pub const PAGE_SIZE: usize = kibibytes(4);
pub const LARGE_PAGE_SIZE: usize = mebibytes(2);
pub const HUGE_PAGE_SIZE: usize = gibibytes(1);

/// Number of descriptors in a translation table for the 4 KiB granule.
pub const N_TABLE_ENTRIES: usize = 512;
pub type PageTable = [Descriptor; N_TABLE_ENTRIES];

/// Returns true if every descriptor in the table is invalid, meaning the table
/// can be freed once unlinked from its parent.
pub fn is_table_unused(table_ptr: NonNull<PageTable>) -> bool {
    unsafe {
        for i in 0..N_TABLE_ENTRIES {
            if (table_ptr.as_ref())[i].is_valid() {
                return false;
            }
        }
    }
    true
}

/// An address space is defined by its two translation table base registers:
/// `TTBR0_EL1` maps the lower half (user space) and `TTBR1_EL1` maps the higher
/// half (kernel space).
#[derive(Debug, Clone, Copy)]
pub struct AddressSpace {
    ttbr0_el1: u64,
    ttbr1_el1: u64,
}

impl AddressSpace {
    pub fn get_ttbr0(&self) -> u64 {
        self.ttbr0_el1
    }

    pub fn get_ttbr1(&self) -> u64 {
        self.ttbr1_el1
    }

    pub fn set_ttbr0(&mut self, ttbr0: u64) {
        self.ttbr0_el1 = ttbr0;
    }

    pub fn set_ttbr1(&mut self, ttbr1: u64) {
        self.ttbr1_el1 = ttbr1;
    }

    /// Map a physical MMIO region into this address space at its higher half
    /// direct map (HHDM) alias, using strongly-ordered Device-nGnRnE memory.
    ///
    /// The region is mapped page-by-page starting at `HHDM_BASE + phys_base`,
    /// so the standard `PAddr::into_hhdm_*` helpers used by device drivers
    /// resolve to these mappings. `phys_base` and `size` are rounded to whole
    /// pages. This is needed because, from Limine base revision 3 onwards, the
    /// bootloader only HHDM-maps real RAM, leaving MMIO unmapped until the
    /// kernel maps it explicitly.
    pub fn map_mmio_region(
        &mut self,
        phys_base: usize,
        size: usize,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        use crate::cpu::isa::interface::memory::address::PhysicalAddress;
        let start = phys_base & !(PAGE_SIZE - 1);
        let end = (phys_base + size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        for phys in (start..end).step_by(PAGE_SIZE) {
            let frame = PAddr::from(phys as u64);
            // The HHDM alias of this physical page is where drivers expect it.
            let hhdm_vaddr = VAddr::from_ptr(unsafe { frame.into_hhdm_ptr::<u8>() });
            let mut walker = walker::Walker::new(self, hhdm_vaddr);
            match walker.map_mmio_page(frame, true) {
                Ok(()) | Err(<MemoryInterfaceImpl as MemoryInterface>::Error::AlreadyMapped) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Map one 4 KiB page of device (MMIO) memory into this address space at
    /// `vaddr`, user-accessible, so a delegated EL0 driver domain can reach a
    /// device's registers directly (architecture doc Phase 8). The frame is
    /// mapped Device-nGnRnE, execute-never, and is not zeroed. Unlike
    /// [`map_mmio_region`](Self::map_mmio_region) the mapping is placed at a
    /// caller-chosen user virtual address rather than the kernel HHDM alias.
    pub fn map_user_mmio_page(
        &mut self,
        vaddr: VAddr,
        frame: PAddr,
        writable: bool,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = walker::Walker::new(self, vaddr);
        walker.map_user_mmio_page(frame, writable)
    }
}

impl AddressSpaceInterface for AddressSpace {
    const HUGE_PAGE_SIZE: usize = HUGE_PAGE_SIZE;
    const LARGE_PAGE_SIZE: usize = LARGE_PAGE_SIZE;
    const PAGE_SIZE: usize = PAGE_SIZE;

    fn get_current() -> Self {
        let ttbr0_el1: u64;
        let ttbr1_el1: u64;
        unsafe {
            asm!("mrs {}, ttbr0_el1", out(reg) ttbr0_el1, options(nomem, nostack, preserves_flags));
            asm!("mrs {}, ttbr1_el1", out(reg) ttbr1_el1, options(nomem, nostack, preserves_flags));
        }
        AddressSpace {
            ttbr0_el1,
            ttbr1_el1,
        }
    }

    fn load(&self) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        unsafe {
            // Update both translation table base registers, then synchronise:
            // DSB to ensure the writes are observable and ISB to flush the
            // pipeline so subsequent instructions use the new translation
            // regime.
            asm!(
                "msr ttbr0_el1, {ttbr0}",
                "msr ttbr1_el1, {ttbr1}",
                "dsb ish",
                "isb",
                ttbr0 = in(reg) self.ttbr0_el1,
                ttbr1 = in(reg) self.ttbr1_el1,
                options(nostack, preserves_flags)
            );
        }
        Ok(())
    }

    fn find_free_region(
        &mut self,
        n_pages: usize,
        range: (VAddr, VAddr),
    ) -> Result<VAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        self.find_free_region_generic(n_pages, range, PAGE_SIZE, |s, v| s.is_mapped(v))
    }

    fn find_free_region_large_aligned(
        &mut self,
        n_large_pages: usize,
        range: (VAddr, VAddr),
    ) -> Result<VAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        if !range.0.is_aligned_to(LARGE_PAGE_SIZE) || !range.1.is_aligned_to(LARGE_PAGE_SIZE) {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::VAddrNotLargePageAligned);
        }
        self.find_free_region_generic(n_large_pages, range, LARGE_PAGE_SIZE, |s, v| {
            s.is_mapped_large_page(v)
        })
    }

    fn find_free_region_huge_aligned(
        &mut self,
        n_huge_pages: usize,
        range: (VAddr, VAddr),
    ) -> Result<VAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        if !range.0.is_aligned_to(HUGE_PAGE_SIZE) || !range.1.is_aligned_to(HUGE_PAGE_SIZE) {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::VAddrNotHugePageAligned);
        }
        self.find_free_region_generic(n_huge_pages, range, HUGE_PAGE_SIZE, |s, v| {
            s.is_mapped_huge_page(v)
        })
    }

    fn map_page(
        &mut self,
        mapping: MemoryMapping,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = walker::Walker::new(self, mapping.vaddr);
        walker.map_page(
            mapping.paddr,
            mapping.page_type.is_writable(),
            mapping.page_type.is_user_accessible(),
            mapping.page_type.is_no_execute(),
        )
    }

    fn map_existing_page(
        &mut self,
        mapping: MemoryMapping,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = walker::Walker::new(self, mapping.vaddr);
        walker.map_existing_page(
            mapping.paddr,
            mapping.page_type.is_writable(),
            mapping.page_type.is_user_accessible(),
            mapping.page_type.is_no_execute(),
        )
    }

    fn unmap_page(
        &mut self,
        vaddr: VAddr,
    ) -> Result<PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        if <VAddr as Into<usize>>::into(vaddr) == 0 {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NullVAddrNotAllowed);
        }
        if vaddr.page_offset() != 0 {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::VAddrNotPageAligned);
        }
        let mut walker = walker::Walker::new(self, vaddr);
        walker.unmap_page()
    }

    fn map_large_page(
        &mut self,
        mapping: MemoryMapping,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        if !mapping.vaddr.is_aligned_to(LARGE_PAGE_SIZE) {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::VAddrNotLargePageAligned);
        }
        let mut walker = walker::Walker::new(self, mapping.vaddr);
        walker.map_large_page(
            mapping.paddr,
            mapping.page_type.is_writable(),
            mapping.page_type.is_user_accessible(),
            mapping.page_type.is_no_execute(),
        )
    }

    fn unmap_large_page(
        &mut self,
        vaddr: VAddr,
    ) -> Result<PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        if <VAddr as Into<usize>>::into(vaddr) == 0 {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NullVAddrNotAllowed);
        }
        if !vaddr.is_aligned_to(LARGE_PAGE_SIZE) {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::VAddrNotLargePageAligned);
        }
        let mut walker = walker::Walker::new(self, vaddr);
        walker.unmap_large_page()
    }

    fn map_huge_page(
        &mut self,
        mapping: MemoryMapping,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        if !mapping.vaddr.is_aligned_to(HUGE_PAGE_SIZE) {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::VAddrNotHugePageAligned);
        }
        let mut walker = walker::Walker::new(self, mapping.vaddr);
        walker.map_huge_page(
            mapping.paddr,
            mapping.page_type.is_writable(),
            mapping.page_type.is_user_accessible(),
            mapping.page_type.is_no_execute(),
        )
    }

    fn unmap_huge_page(
        &mut self,
        vaddr: VAddr,
    ) -> Result<PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        if <VAddr as Into<usize>>::into(vaddr) == 0 {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NullVAddrNotAllowed);
        }
        if !vaddr.is_aligned_to(HUGE_PAGE_SIZE) {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::VAddrNotHugePageAligned);
        }
        let mut walker = walker::Walker::new(self, vaddr);
        walker.unmap_huge_page()
    }

    fn is_mapped(
        &mut self,
        vaddr: VAddr,
    ) -> Result<bool, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = walker::Walker::new(self, vaddr);
        match walker.walk() {
            Ok(_) => Ok(true),
            Err(<MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped) => {
                self.is_mapped_large_page(vaddr).and_then(|large| {
                    if large {
                        Ok(true)
                    } else {
                        self.is_mapped_huge_page(vaddr)
                    }
                })
            }
            Err(e) => Err(e),
        }
    }

    fn is_mapped_large_page(
        &mut self,
        vaddr: VAddr,
    ) -> Result<bool, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = walker::Walker::new(self, vaddr);
        match walker.walk_large_page() {
            Ok(_) => Ok(true),
            Err(<MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn is_mapped_huge_page(
        &mut self,
        vaddr: VAddr,
    ) -> Result<bool, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = walker::Walker::new(self, vaddr);
        match walker.walk_huge_page() {
            Ok(_) => Ok(true),
            Err(<MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn translate_address(
        &mut self,
        vaddr: VAddr,
    ) -> Result<PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = walker::Walker::new(self, vaddr);
        walker.translate()
    }
}

impl AddressSpace {
    /// Shared free-region search used for the standard, large, and huge page
    /// variants. Scans `range` in `stride`-sized steps looking for `n` slots
    /// that are all unmapped according to `is_mapped`.
    fn find_free_region_generic(
        &mut self,
        n: usize,
        range: (VAddr, VAddr),
        stride: usize,
        mut is_mapped: impl FnMut(
            &mut Self,
            VAddr,
        )
            -> Result<bool, <MemoryInterfaceImpl as MemoryInterface>::Error>,
    ) -> Result<VAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut page_iter = (range.0..range.1).step_by(stride);
        while let Some(base) = page_iter.next() {
            for nth in 0..n {
                let curr = base + (nth * stride) as isize;
                if range.1 - curr < (n * stride) as isize {
                    return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable);
                }
                if is_mapped(self, curr)? {
                    if page_iter.advance_by(nth).is_err() {
                        return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable);
                    }
                    break;
                }
                if nth == n - 1 {
                    return Ok(base);
                }
            }
        }
        Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable)
    }
}
