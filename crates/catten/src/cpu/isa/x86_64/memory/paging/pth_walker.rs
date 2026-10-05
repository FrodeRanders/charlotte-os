//! # Page Table Hierarchy Walker
//!
//! This module implements the page table hierarchy walker for the x86_64 architecture.
//! This structure performs the actual page table walk, translating virtual addresses to physical
//! addresses, mapping pages, and unmapping pages as well as adding and removing page table entries
//! and page tables as needed.

use super::{
    CR3_ADDRESS_MASK,
    PAGE_SIZE,
};
use crate::{
    cpu::isa::{
        interface::memory::{
            MemoryInterface,
            address::VirtualAddress,
        },
        x86_64::memory::address::{
            paddr::PAddr,
            vaddr::VAddr,
        },
    },
    memory::PHYSICAL_FRAME_ALLOCATOR,
};
type WalkerError = <super::MemoryInterfaceImpl as MemoryInterface>::Error;
type WalkerResult<T> = Result<T, WalkerError>;

pub struct PthWalker<'vas> {
    pub address_space: &'vas mut super::AddressSpace,
    pub vaddr: VAddr,
    pub pml4_ptr: *mut super::PageTable,
    pub pdpt_ptr: *mut super::PageTable,
    pub pd_ptr: *mut super::PageTable,
    pub pt_ptr: *mut super::PageTable,
    pub page_frame_ptr: *mut [u8; super::PAGE_SIZE],
}

impl<'vas> PthWalker<'vas> {
    pub fn new(address_space: &'vas mut super::AddressSpace, vaddr: VAddr) -> Self {
        Self {
            address_space,
            vaddr,
            pml4_ptr: core::ptr::null_mut(),
            pdpt_ptr: core::ptr::null_mut(),
            pd_ptr: core::ptr::null_mut(),
            pt_ptr: core::ptr::null_mut(),
            page_frame_ptr: core::ptr::null_mut(),
        }
    }

    fn unmapped_error() -> WalkerError {
        <super::MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped
    }

    fn already_mapped_error() -> WalkerError {
        <super::MemoryInterfaceImpl as MemoryInterface>::Error::AlreadyMapped
    }

    fn root_table_ptr(&self) -> WalkerResult<*mut super::PageTable> {
        let base = self.address_space.cr3 & CR3_ADDRESS_MASK;
        if base == 0 {
            return Err(Self::unmapped_error());
        }
        Ok(PAddr::try_from(base as usize).unwrap().into())
    }

    fn walk_next_level(
        &self,
        table_ptr: *mut super::PageTable,
        index: usize,
        page_size_must_be_set: bool,
        page_size_must_be_clear: bool,
    ) -> WalkerResult<*mut super::PageTable> {
        unsafe {
            let pte = &mut (*table_ptr)[index];
            if !pte.is_present()
                || (page_size_must_be_set && !pte.get_page_size())
                || (page_size_must_be_clear && pte.get_page_size())
            {
                return Err(Self::unmapped_error());
            }
            let Ok(frame) = pte.try_get_frame() else {
                return Err(Self::unmapped_error());
            };
            Ok(frame.into())
        }
    }

    fn set_table_entry(
        entry: &mut super::pte::PageTableEntry,
        frame: PAddr,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
        page_size: bool,
    ) {
        let mut prepared = *entry;
        prepared
            .set_frame(frame)
            .set_present(true)
            .set_writable(writable)
            .set_user_accessible(user_accessible)
            .set_execute_disabled(no_execute)
            .set_page_size(page_size);
        Self::publish_entry(entry, prepared);
    }

    fn publish_entry(entry: &mut super::pte::PageTableEntry, prepared: super::pte::PageTableEntry) {
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        // One aligned entry store publishes a completely initialized table
        // link. Do not expose Present before the other permission bits.
        unsafe { core::ptr::write_volatile(entry, prepared) };
    }

    fn allocate_and_link_table(
        &mut self,
        parent_table_ptr: *mut super::PageTable,
        parent_index: usize,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
    ) -> WalkerResult<*mut super::PageTable> {
        let new_table = PHYSICAL_FRAME_ALLOCATOR.lock().allocate_frame()?;
        unsafe {
            let new_table_ptr: *mut super::PageTable = new_table.into();
            // Scrub before publishing: hardware walks are not serialized by
            // the software address-space lock.
            core::ptr::write_bytes(new_table_ptr.cast::<u8>(), 0, PAGE_SIZE);
            Self::set_table_entry(
                &mut (*parent_table_ptr)[parent_index],
                new_table,
                writable,
                user_accessible,
                no_execute,
                false,
            );
            Ok(new_table_ptr)
        }
    }

    fn ensure_pml4(&mut self) -> WalkerResult<*mut super::PageTable> {
        if self.pml4_ptr.is_null() {
            // try_new_user owns initial root preparation. A walker must never
            // install an uninitialized CR3 or scrub an existing live root.
            self.pml4_ptr = self.root_table_ptr()?;
        }
        Ok(self.pml4_ptr)
    }

    fn prepare_map_walk_result(walk_result: WalkerResult<()>) -> WalkerResult<()> {
        match walk_result {
            Ok(_) => Err(Self::already_mapped_error()),
            Err(<super::MemoryInterfaceImpl as MemoryInterface>::Error::Unmapped) => Ok(()),
            Err(other) => Err(other),
        }
    }

    pub fn walk(&mut self) -> WalkerResult<()> {
        self.pml4_ptr = self.root_table_ptr()?;
        self.pdpt_ptr =
            self.walk_next_level(self.pml4_ptr, self.vaddr.pml4_index(), false, false)?;
        self.pd_ptr = self.walk_next_level(self.pdpt_ptr, self.vaddr.pdpt_index(), false, true)?;
        self.pt_ptr = self.walk_next_level(self.pd_ptr, self.vaddr.pd_index(), false, true)?;
        self.page_frame_ptr = self
            .walk_next_level(self.pt_ptr, self.vaddr.pt_index(), false, false)?
            .cast::<[u8; super::PAGE_SIZE]>();

        Ok(())
    }

    pub fn walk_large_page(&mut self) -> WalkerResult<()> {
        self.pml4_ptr = self.root_table_ptr()?;
        self.pdpt_ptr =
            self.walk_next_level(self.pml4_ptr, self.vaddr.pml4_index(), false, false)?;
        self.pd_ptr = self.walk_next_level(self.pdpt_ptr, self.vaddr.pdpt_index(), false, false)?;
        self.pt_ptr = core::ptr::null_mut();
        self.page_frame_ptr = self
            .walk_next_level(self.pd_ptr, self.vaddr.pd_index(), true, false)?
            .cast::<[u8; super::PAGE_SIZE]>();

        Ok(())
    }

    pub fn walk_huge_page(&mut self) -> WalkerResult<()> {
        self.pml4_ptr = self.root_table_ptr()?;
        self.pdpt_ptr =
            self.walk_next_level(self.pml4_ptr, self.vaddr.pml4_index(), false, false)?;
        self.pd_ptr = core::ptr::null_mut();
        self.pt_ptr = core::ptr::null_mut();
        self.page_frame_ptr = self
            .walk_next_level(self.pdpt_ptr, self.vaddr.pdpt_index(), true, false)?
            .cast::<[u8; super::PAGE_SIZE]>();

        Ok(())
    }

    pub fn map_page(
        &mut self,
        frame: PAddr,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
        pat_index: u8,
    ) -> WalkerResult<()> {
        self.map_page_with_attrs(frame, writable, user_accessible, no_execute, pat_index, true)
    }

    /// Map a page that is already owned and populated (e.g. a memory-object
    /// frame being moved into another address space) without zeroing it.
    pub fn map_existing_page(
        &mut self,
        frame: PAddr,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
        pat_index: u8,
    ) -> WalkerResult<()> {
        self.map_page_with_attrs(frame, writable, user_accessible, no_execute, pat_index, false)
    }

    fn map_page_with_attrs(
        &mut self,
        frame: PAddr,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
        pat_index: u8,
        zero: bool,
    ) -> WalkerResult<()> {
        Self::prepare_map_walk_result(self.walk())?;
        self.ensure_pml4()?;
        if self.pdpt_ptr.is_null() {
            // Allocate a new page table for the PDPT
            self.pdpt_ptr = self.allocate_and_link_table(
                self.pml4_ptr,
                self.vaddr.pml4_index(),
                writable,
                user_accessible,
                no_execute,
            )?;
        }
        if self.pd_ptr.is_null() {
            self.pd_ptr = self.allocate_and_link_table(
                self.pdpt_ptr,
                self.vaddr.pdpt_index(),
                writable,
                user_accessible,
                no_execute,
            )?;
        }
        if self.pt_ptr.is_null() {
            let pde = unsafe { &(*self.pd_ptr)[self.vaddr.pd_index()] };
            if pde.is_present() {
                return Err(<super::MemoryInterfaceImpl as MemoryInterface>::Error::AlreadyMapped);
            }
            // Allocate a new page table for the PT
            self.pt_ptr = self.allocate_and_link_table(
                self.pd_ptr,
                self.vaddr.pd_index(),
                writable,
                user_accessible,
                no_execute,
            )?;
        }
        // A shared subtree may contain a mix of read-only (e.g. code) and
        // writable (e.g. data) pages. Intermediate entries are created with the
        // flags of the first page mapped beneath them, so widening them here
        // ensures a later writable/user mapping is not silently made read-only
        // (or supervisor-only) by a stale parent bit.
        if writable || user_accessible {
            unsafe {
                let pml4e = &mut (*self.pml4_ptr)[self.vaddr.pml4_index()];
                if writable {
                    pml4e.set_writable(true);
                }
                if user_accessible {
                    pml4e.set_user_accessible(true);
                }
                let pdpte = &mut (*self.pdpt_ptr)[self.vaddr.pdpt_index()];
                if writable {
                    pdpte.set_writable(true);
                }
                if user_accessible {
                    pdpte.set_user_accessible(true);
                }
                let pde = &mut (*self.pd_ptr)[self.vaddr.pd_index()];
                if writable {
                    pde.set_writable(true);
                }
                if user_accessible {
                    pde.set_user_accessible(true);
                }
            }
        }
        // Map the page frame
        unsafe {
            if zero {
                core::ptr::write_bytes(<PAddr as Into<*mut u8>>::into(frame), 0, PAGE_SIZE);
            }
            let mut prepared = super::pte::PageTableEntry::new(
                true,
                writable,
                user_accessible,
                pat_index,
                false,
                frame,
            );
            prepared.set_execute_disabled(no_execute);
            Self::publish_entry(&mut (*self.pt_ptr)[self.vaddr.pt_index()], prepared);
        }
        // Do NOT reload CR3 here: the address space being mapped may not be the
        // one currently active (e.g. mapping pages into a freshly created user
        // address space while the kernel AS is live). A non-current address
        // space is flushed when its CR3 is next loaded; a current one is
        // flushed below by invalidating the single translation.
        unsafe {
            // Get rid of any stale TLB entries referring to the linear address space
            // aperture into which the newly allocated page frame has been mapped.
            core::arch::asm!("invlpg [{}]", in(reg) self.vaddr.into_ptr::<u8>());
        }

        Ok(())
    }

    pub fn unmap_page(&mut self) -> WalkerResult<PAddr> {
        match self.walk() {
            Ok(_) => {
                unsafe {
                    // get the return value
                    let pte = &raw mut (*self.pt_ptr)[self.vaddr.pt_index()];
                    let Ok(paddr) = (*pte).try_get_frame() else {
                        return Err(Self::unmapped_error());
                    };
                    if (*pte).is_present() {
                        // We do not deallocate the page frame here, as it is the responsibility of
                        // the VMM client calling this function to deallocate the frame if they need
                        // to.
                        (*pte).set_present(false);
                    }

                    // Keep empty intermediate tables linked and owned until
                    // address-space teardown. A local INVLPG does not quiesce
                    // remote walkers, and this call may hold IRQ-masking
                    // locks that preclude a synchronous cross-LP rendezvous.
                    // Remaps reuse the retained tree; Drop reclaims it after
                    // the domain's threads and translations have retired.
                    // Invalidate the removed translation locally. The cross-LP
                    // shootdown is the responsibility of the VMM client, which
                    // knows the address-space identity and can rendezvous once
                    // per logical operation rather than per page.
                    core::arch::asm!("invlpg [{}]", in(reg) self.vaddr.into_ptr::<u8>());
                    Ok(paddr)
                }
            }
            Err(other) => Err(other),
        }
    }

    pub fn map_large_page(
        &mut self,
        frame: PAddr,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
        pat_index: u8,
    ) -> WalkerResult<()> {
        Self::prepare_map_walk_result(self.walk_large_page())?;
        self.ensure_pml4()?;
        if self.pdpt_ptr.is_null() {
            // Allocate a new page table for the PDPT
            self.pdpt_ptr = self.allocate_and_link_table(
                self.pml4_ptr,
                self.vaddr.pml4_index(),
                writable,
                user_accessible,
                no_execute,
            )?;
        }
        if self.pd_ptr.is_null() {
            // Allocate a new page table for the PD
            self.pd_ptr = self.allocate_and_link_table(
                self.pdpt_ptr,
                self.vaddr.pdpt_index(),
                writable,
                user_accessible,
                no_execute,
            )?;
        }
        unsafe {
            if (*self.pd_ptr)[self.vaddr.pd_index()].is_present() {
                return Err(Self::already_mapped_error());
            }
            // Map the large page frame directly in the Page Directory (PML2) with the PS
            // bit set
            let mut prepared = super::pte::PageTableEntry::new_large_huge(
                true,
                writable,
                user_accessible,
                pat_index,
                false,
                frame,
            );
            prepared.set_execute_disabled(no_execute);
            Self::publish_entry(&mut (*self.pd_ptr)[self.vaddr.pd_index()], prepared);
        }
        Ok(())
    }

    pub fn unmap_large_page(&mut self) -> WalkerResult<PAddr> {
        self.walk_large_page()?;
        unsafe {
            let pde = &raw mut (*self.pd_ptr)[self.vaddr.pd_index()];
            let Ok(paddr) = (*pde).try_get_frame() else {
                return Err(Self::unmapped_error());
            };
            (*pde).set_present(false);
            core::arch::asm!("invlpg [{}]", in(reg) self.vaddr.into_ptr::<u8>());
            Ok(paddr)
        }
    }

    pub fn map_huge_page(
        &mut self,
        frame: PAddr,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
        pat_index: u8,
    ) -> WalkerResult<()> {
        Self::prepare_map_walk_result(self.walk_huge_page())?;
        self.ensure_pml4()?;
        if self.pdpt_ptr.is_null() {
            // Allocate a new page table for the PDPT
            self.pdpt_ptr = self.allocate_and_link_table(
                self.pml4_ptr,
                self.vaddr.pml4_index(),
                writable,
                user_accessible,
                no_execute,
            )?;
        }
        unsafe {
            if (*self.pdpt_ptr)[self.vaddr.pdpt_index()].is_present() {
                return Err(Self::already_mapped_error());
            }
            // Map the huge page frame directly in the Page Directory Pointer Table (PML3)
            // with the PS bit set
            let mut prepared = super::pte::PageTableEntry::new_large_huge(
                true,
                writable,
                user_accessible,
                pat_index,
                false,
                frame,
            );
            prepared.set_execute_disabled(no_execute);
            Self::publish_entry(&mut (*self.pdpt_ptr)[self.vaddr.pdpt_index()], prepared);
        }
        Ok(())
    }

    pub fn unmap_huge_page(&mut self) -> WalkerResult<PAddr> {
        self.walk_huge_page()?;
        unsafe {
            let pdpte = &raw mut (*self.pdpt_ptr)[self.vaddr.pdpt_index()];
            let Ok(paddr) = (*pdpte).try_get_frame() else {
                return Err(Self::unmapped_error());
            };
            (*pdpte).set_present(false);
            core::arch::asm!("invlpg [{}]", in(reg) self.vaddr.into_ptr::<u8>());
            Ok(paddr)
        }
    }
}
