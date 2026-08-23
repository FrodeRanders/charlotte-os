pub mod pte;
pub mod pth_walker;
use core::{
    arch::asm,
    iter::Iterator,
    ptr::NonNull,
};

use super::{
    MemoryInterfaceImpl,
    address::vaddr::VAddr,
};
use crate::{
    cpu::isa::interface::memory::{
        AddressSpaceInterface,
        MemoryInterface,
        MemoryMapping,
        address::Address,
    },
    klib::size::{
        gibibytes,
        kibibytes,
        mebibytes,
    },
    memory::{
        KERNEL_ASID,
        PAddr,
        PHYSICAL_FRAME_ALLOCATOR,
    },
};

pub const PAGE_SIZE: usize = 4096;
pub const N_PAGE_TABLE_ENTRIES: usize = 512;
pub type PageTable = [pte::PageTableEntry; N_PAGE_TABLE_ENTRIES];

/// Mask of the address bits of CR3 (the top-level page-table physical base).
pub const CR3_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

/// The logical [`AddressSpaceId`](crate::memory::AddressSpaceId) of the thread
/// currently executing on each logical processor, maintained by the context
/// switch and read by the SYSCALL entry path to attribute the caller's
/// syscalls. Mirror of the AArch64 [`CURRENT_LOGICAL_ASID`].
pub static CURRENT_LOGICAL_ASID: [core::sync::atomic::AtomicUsize;
    crate::cpu::scheduler::system_scheduler::MAX_TRACKED_LPS] =
    [const { core::sync::atomic::AtomicUsize::new(KERNEL_ASID) };
        crate::cpu::scheduler::system_scheduler::MAX_TRACKED_LPS];

pub fn is_pagetable_unused(table_ptr: NonNull<PageTable>) -> bool {
    unsafe {
        for i in 0..N_PAGE_TABLE_ENTRIES {
            if (table_ptr.as_ref())[i].is_present() {
                return false;
            }
        }
    }
    true
}

pub struct AddressSpace {
    // control register 3 i.e. top level page table base register
    cr3: u64,
    owns_root: bool,
}

impl AddressSpace {
    /// Construct an isolated user address space sharing only the kernel
    /// (higher-half) mappings.
    ///
    /// A fresh, zeroed PML4 is allocated and the kernel's top-level entries
    /// (indices 256..512, i.e. the whole higher half) are copied from the
    /// active translation root. Because those entries point at the same shared
    /// PML3/PML2/PML1 tables, later kernel mapping changes below the PML4 level
    /// remain visible in every address space; the user half (indices 0..256)
    /// starts empty so ring-3 code can only reach what the loader explicitly
    /// maps. The new address space is not installed: callers map into it
    /// through the address-space table and it becomes active only when a thread
    /// switches CR3 to it.
    pub fn new_user() -> Self {
        let current = Self::get_current();
        let new_pml4 = PHYSICAL_FRAME_ALLOCATOR
            .lock()
            .allocate_frame()
            .expect("Failed to allocate a PML4 frame for a user address space.");
        let new_pml4_ptr: *mut PageTable = new_pml4.into();
        unsafe {
            core::ptr::write_bytes(new_pml4_ptr.cast::<u8>(), 0, PAGE_SIZE);
            let cur_pml4: *const PageTable =
                PAddr::try_from((current.cr3 & CR3_ADDRESS_MASK) as usize).unwrap().into();
            for index in 256..N_PAGE_TABLE_ENTRIES {
                (*new_pml4_ptr)[index] = (*cur_pml4)[index];
            }
        }
        AddressSpace {
            cr3: <PAddr as Into<u64>>::into(new_pml4) & CR3_ADDRESS_MASK,
            owns_root: true,
        }
    }

    pub fn get_cr3(&self) -> u64 {
        self.cr3
    }

    /// Ensure an MMIO region is reachable through its kernel HHDM alias.
    ///
    /// Limine may already have installed the mapping; in that case this is
    /// idempotent. New mappings use the architecture-neutral MMIO page type
    /// (writable, supervisor-only, execute-disabled).
    pub fn map_mmio_region(
        &mut self,
        phys_base: usize,
        size: usize,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        use crate::{
            cpu::isa::interface::memory::address::{
                PhysicalAddress,
                VirtualAddress,
            },
            memory::linear::PageType,
        };

        let start = phys_base & !(PAGE_SIZE - 1);
        let end =
            phys_base.checked_add(size).and_then(|end| end.checked_add(PAGE_SIZE - 1)).ok_or(
                <MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable,
            )? & !(PAGE_SIZE - 1);
        for phys in (start..end).step_by(PAGE_SIZE) {
            let frame = PAddr::from(phys as u64);
            let mapping = MemoryMapping {
                vaddr: VAddr::from_ptr(unsafe { frame.into_hhdm_ptr::<u8>() }),
                paddr: frame,
                page_type: PageType::Mmio,
            };
            // Never zero an MMIO frame: writing zeros to a device register
            // block would corrupt the hardware. `map_existing_page` installs
            // the mapping without touching the target.
            match self.map_existing_page(mapping) {
                Ok(()) | Err(<MemoryInterfaceImpl as MemoryInterface>::Error::AlreadyMapped) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

impl AddressSpaceInterface for AddressSpace {
    const HUGE_PAGE_SIZE: usize = gibibytes(1);
    const LARGE_PAGE_SIZE: usize = mebibytes(2);
    const PAGE_SIZE: usize = kibibytes(4);

    fn get_current() -> Self {
        let cr3: u64;
        unsafe {
            asm!("mov {}, cr3", out(reg) cr3);
        }
        AddressSpace {
            cr3,
            owns_root: false,
        }
    }

    fn load(&self) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        unsafe {
            // Set the top level page table base register
            asm!("mov cr3, {}", in(reg) self.cr3);
        }
        Ok(())
    }

    fn find_free_region(
        &mut self,
        n_pages: usize,
        range: (VAddr, VAddr),
    ) -> Result<
        <MemoryInterfaceImpl as MemoryInterface>::VAddr,
        <MemoryInterfaceImpl as MemoryInterface>::Error,
    > {
        let mut page_iter = (range.0..range.1).step_by(PAGE_SIZE);
        while let Some(base) = page_iter.next() {
            //logln!("Checking base address: {:?}", base);
            for nth_page in 0..n_pages {
                let curr_page = base + ((nth_page * PAGE_SIZE) as isize);
                //logln!("Checking page: {:?}", curr_page);
                if range.1 - curr_page < (n_pages * PAGE_SIZE) as isize {
                    return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable);
                }
                if self.is_mapped(curr_page)? {
                    match page_iter.advance_by(nth_page) {
                        Ok(_) => {
                            //logln!("Page {:?} is already mapped, skipping to next base address.", curr_page);
                            break;
                        }
                        Err(_) => return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable),
                    }
                }
                if nth_page == n_pages - 1 {
                    return Ok(base);
                }
            }
        }
        Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable)
    }

    fn find_free_region_large_aligned(
        &mut self,
        n_large_pages: usize,
        range: (VAddr, VAddr),
    ) -> Result<VAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        if !range.0.is_aligned_to(Self::LARGE_PAGE_SIZE)
            || !range.1.is_aligned_to(Self::LARGE_PAGE_SIZE)
        {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::VAddrNotLargePageAligned);
        }
        let mut page_iter = (range.0..range.1).step_by(Self::LARGE_PAGE_SIZE);
        while let Some(base) = page_iter.next() {
            //logln!("Checking base address: {:?}", base);
            for nth_page in 0..n_large_pages {
                let curr_page = base + ((nth_page * Self::LARGE_PAGE_SIZE) as isize);
                //logln!("Checking large page: {:?}", curr_page);
                if range.1 - curr_page < (n_large_pages * Self::LARGE_PAGE_SIZE) as isize {
                    return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable);
                }
                if self.is_mapped_large_page(curr_page)? {
                    match page_iter.advance_by(nth_page) {
                        Ok(_) => {
                            //logln!("Large page {:?} is already mapped, skipping to next base address.", curr_page);
                            break;
                        }
                        Err(_) => return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable),
                    }
                }
                if nth_page == n_large_pages - 1 {
                    return Ok(base);
                }
            }
        }
        Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable)
    }

    fn find_free_region_huge_aligned(
        &mut self,
        n_huge_pages: usize,
        range: (VAddr, VAddr),
    ) -> Result<VAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        if !range.0.is_aligned_to(Self::HUGE_PAGE_SIZE)
            || !range.1.is_aligned_to(Self::HUGE_PAGE_SIZE)
        {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::VAddrNotHugePageAligned);
        }
        let mut page_iter = (range.0..range.1).step_by(Self::HUGE_PAGE_SIZE);
        while let Some(base) = page_iter.next() {
            //logln!("Checking base address: {:?}", base);
            for nth_page in 0..n_huge_pages {
                let curr_page = base + ((nth_page * Self::HUGE_PAGE_SIZE) as isize);
                //logln!("Checking huge page: {:?}", curr_page);
                if range.1 - curr_page < (n_huge_pages * Self::HUGE_PAGE_SIZE) as isize {
                    return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable);
                }
                if self.is_mapped_huge_page(curr_page)? {
                    match page_iter.advance_by(nth_page) {
                        Ok(_) => {
                            //logln!("Huge page {:?} is already mapped, skipping to next base address.", curr_page);
                            break;
                        }
                        Err(_) => return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable),
                    }
                }
                if nth_page == n_huge_pages - 1 {
                    return Ok(base);
                }
            }
        }
        Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NoRequestedVAddrRegionAvailable)
    }

    fn map_page(
        &mut self,
        mapping: MemoryMapping,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = pth_walker::PthWalker::new(self, mapping.vaddr);
        walker.map_page(
            mapping.paddr,
            mapping.page_type.is_writable(),
            mapping.page_type.is_user_accessible(),
            mapping.page_type.is_no_execute(),
        )?;
        Ok(())
    }

    fn map_existing_page(
        &mut self,
        mapping: MemoryMapping,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = pth_walker::PthWalker::new(self, mapping.vaddr);
        walker.map_existing_page(
            mapping.paddr,
            mapping.page_type.is_writable(),
            mapping.page_type.is_user_accessible(),
            mapping.page_type.is_no_execute(),
        )?;
        Ok(())
    }

    fn unmap_page(
        &mut self,
        vaddr: <MemoryInterfaceImpl as MemoryInterface>::VAddr,
    ) -> Result<PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        if <VAddr as Into<usize>>::into(vaddr) == 0 {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NullVAddrNotAllowed);
        }
        if vaddr.page_offset() != 0 {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::VAddrNotPageAligned);
        }
        let mut walker = pth_walker::PthWalker::new(self, vaddr);
        walker.unmap_page()
    }

    fn map_large_page(
        &mut self,
        mapping: MemoryMapping,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = pth_walker::PthWalker::new(self, mapping.vaddr);
        walker.map_large_page(
            mapping.paddr,
            mapping.page_type.is_writable(),
            mapping.page_type.is_user_accessible(),
            mapping.page_type.is_no_execute(),
        )?;
        Ok(())
    }

    fn unmap_large_page(
        &mut self,
        vaddr: <MemoryInterfaceImpl as MemoryInterface>::VAddr,
    ) -> Result<PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        if <VAddr as Into<usize>>::into(vaddr) == 0 {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NullVAddrNotAllowed);
        }
        if !vaddr.is_aligned_to(Self::LARGE_PAGE_SIZE) {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::VAddrNotLargePageAligned);
        }
        let mut walker = pth_walker::PthWalker::new(self, vaddr);
        walker.unmap_large_page()
    }

    fn map_huge_page(
        &mut self,
        mapping: MemoryMapping,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = pth_walker::PthWalker::new(self, mapping.vaddr);
        walker.map_huge_page(
            mapping.paddr,
            mapping.page_type.is_writable(),
            mapping.page_type.is_user_accessible(),
            mapping.page_type.is_no_execute(),
        )?;
        Ok(())
    }

    fn unmap_huge_page(
        &mut self,
        vaddr: <MemoryInterfaceImpl as MemoryInterface>::VAddr,
    ) -> Result<PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        if <VAddr as Into<usize>>::into(vaddr) == 0 {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::NullVAddrNotAllowed);
        }
        if !vaddr.is_aligned_to(Self::HUGE_PAGE_SIZE) {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::VAddrNotHugePageAligned);
        }
        let mut walker = pth_walker::PthWalker::new(self, vaddr);
        walker.unmap_huge_page()
    }

    fn is_mapped(
        &mut self,
        vaddr: <MemoryInterfaceImpl as MemoryInterface>::VAddr,
    ) -> Result<bool, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = pth_walker::PthWalker::new(self, vaddr);
        match walker.walk() {
            Ok(_) => Ok(true),
            Err(<MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped) => {
                match walker.walk_large_page() {
                    Ok(_) => Ok(true),
                    Err(<MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped) => {
                        match walker.walk_huge_page() {
                            Ok(_) => Ok(true),
                            Err(<MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped) => {
                                Ok(false)
                            }
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    fn is_mapped_large_page(
        &mut self,
        vaddr: VAddr,
    ) -> Result<bool, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = pth_walker::PthWalker::new(self, vaddr);
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
        let mut walker = pth_walker::PthWalker::new(self, vaddr);
        match walker.walk_huge_page() {
            Ok(_) => Ok(true),
            Err(<MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn translate_address(
        &mut self,
        vaddr: super::address::vaddr::VAddr,
    ) -> Result<super::address::paddr::PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = pth_walker::PthWalker::new(self, vaddr);
        match walker.walk() {
            Ok(_) => {
                let mut paddr = unsafe { (*(walker.pt_ptr))[vaddr.pt_index()].try_get_frame()? };
                paddr += vaddr.page_offset();
                Ok(paddr)
            }
            Err(<MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped) => {
                match walker.walk_large_page() {
                    Ok(_) => {
                        let mut paddr =
                            unsafe { (*(walker.pd_ptr))[vaddr.pd_index()].try_get_frame()? };
                        paddr += vaddr.page_offset() + (vaddr.pt_index() * PAGE_SIZE);
                        Ok(paddr)
                    }
                    Err(<MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped) => {
                        walker.walk_huge_page()?;
                        let mut paddr =
                            unsafe { (*(walker.pdpt_ptr))[vaddr.pdpt_index()].try_get_frame()? };
                        paddr += vaddr.page_offset()
                            + (vaddr.pt_index() * PAGE_SIZE)
                            + (vaddr.pd_index() * Self::LARGE_PAGE_SIZE);
                        Ok(paddr)
                    }
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    fn translate_user_writable_address(
        &mut self,
        vaddr: super::address::vaddr::VAddr,
    ) -> Result<super::address::paddr::PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = pth_walker::PthWalker::new(self, vaddr);
        let (frame, offset, permitted) = match walker.walk() {
            Ok(()) => unsafe {
                let entries = [
                    &(*walker.pml4_ptr)[vaddr.pml4_index()],
                    &(*walker.pdpt_ptr)[vaddr.pdpt_index()],
                    &(*walker.pd_ptr)[vaddr.pd_index()],
                    &(*walker.pt_ptr)[vaddr.pt_index()],
                ];
                (
                    entries[3].try_get_frame()?,
                    vaddr.page_offset(),
                    entries.iter().all(|entry| entry.is_user_accessible() && entry.is_writable()),
                )
            },
            Err(<MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped) => {
                match walker.walk_large_page() {
                    Ok(()) => unsafe {
                        let entries = [
                            &(*walker.pml4_ptr)[vaddr.pml4_index()],
                            &(*walker.pdpt_ptr)[vaddr.pdpt_index()],
                            &(*walker.pd_ptr)[vaddr.pd_index()],
                        ];
                        (
                            entries[2].try_get_frame()?,
                            vaddr.page_offset() + vaddr.pt_index() * PAGE_SIZE,
                            entries
                                .iter()
                                .all(|entry| entry.is_user_accessible() && entry.is_writable()),
                        )
                    },
                    Err(<MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped) => {
                        walker.walk_huge_page()?;
                        unsafe {
                            let entries = [
                                &(*walker.pml4_ptr)[vaddr.pml4_index()],
                                &(*walker.pdpt_ptr)[vaddr.pdpt_index()],
                            ];
                            (
                                entries[1].try_get_frame()?,
                                vaddr.page_offset()
                                    + vaddr.pt_index() * PAGE_SIZE
                                    + vaddr.pd_index() * Self::LARGE_PAGE_SIZE,
                                entries
                                    .iter()
                                    .all(|entry| entry.is_user_accessible() && entry.is_writable()),
                            )
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        if !permitted {
            return Err(<MemoryInterfaceImpl as MemoryInterface>::Error::PermissionDenied);
        }
        Ok(frame + offset)
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        if !self.owns_root || self.cr3 & CR3_ADDRESS_MASK == 0 {
            return;
        }

        unsafe fn free_table(table: *mut PageTable, level: u8) {
            for entry in unsafe { &mut *table } {
                if !entry.is_present() || level > 1 && entry.get_page_size() {
                    continue;
                }
                if level > 1 {
                    let child_frame = entry.try_get_frame().expect("present page-table entry");
                    let child: *mut PageTable = child_frame.into();
                    unsafe { free_table(child, level - 1) };
                    PHYSICAL_FRAME_ALLOCATOR
                        .lock()
                        .deallocate_frame(child_frame)
                        .expect("failed to release user page-table frame");
                }
                entry.set_present(false);
            }
        }

        let root_frame = PAddr::try_from((self.cr3 & CR3_ADDRESS_MASK) as usize)
            .expect("owned CR3 root is not a physical frame");
        let root: *mut PageTable = root_frame.into();
        // The upper half is shared with the kernel and must never be traversed
        // or released by a user address-space lifetime.
        unsafe {
            for entry in &mut (&mut *root)[..256] {
                if entry.is_present() {
                    let child_frame = entry.try_get_frame().expect("present PML4 entry");
                    free_table(child_frame.into(), 3);
                    PHYSICAL_FRAME_ALLOCATOR
                        .lock()
                        .deallocate_frame(child_frame)
                        .expect("failed to release user PDPT frame");
                    entry.set_present(false);
                }
            }
        }
        PHYSICAL_FRAME_ALLOCATOR
            .lock()
            .deallocate_frame(root_frame)
            .expect("failed to release user PML4 frame");
        self.owns_root = false;
    }
}
