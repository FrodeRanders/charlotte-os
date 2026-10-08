//! # AArch64 Page Table Walker
//!
//! Walks the four level (L0-L3) stage 1 translation table hierarchy for the
//! 4 KiB granule, translating virtual addresses and installing or removing
//! mappings. It mirrors the role of the x86-64 `pth_walker` but works with the
//! AArch64 translation table descriptor format and the TTBR0/TTBR1 split.
//!
//! Level to VA-index mapping (4 KiB granule, 48-bit VA):
//! - L0 <- `VAddr::pml4_index` (bits 47:39)
//! - L1 <- `VAddr::pdpt_index` (bits 38:30) — a block here maps a 1 GiB huge page
//! - L2 <- `VAddr::pd_index`   (bits 29:21) — a block here maps a 2 MiB large page
//! - L3 <- `VAddr::pt_index`   (bits 20:12) — a page here maps a 4 KiB page
//!
//! Page tables are reached through the higher half direct map (HHDM) that Limine
//! establishes: a physical frame address converts to a usable pointer via
//! `PAddr: Into<*mut T>`.

use super::{
    AddressSpace,
    PAGE_SIZE,
    PageTable,
    descriptor::{
        Descriptor,
        MAIR_IDX_DEVICE,
    },
};
use crate::{
    cpu::isa::{
        aarch64::memory::{
            address::{
                paddr::PAddr,
                vaddr::VAddr,
            },
            tlb,
        },
        interface::memory::{
            AddressSpaceInterface,
            MemoryInterface,
        },
    },
    memory::translation::{
        PreparingTable,
        TableScope,
    },
};

type WalkerError = <super::super::MemoryInterfaceImpl as MemoryInterface>::Error;
type WalkerResult<T> = Result<T, WalkerError>;

/// TTBRn_EL1 BADDR occupies bits [47:12] for the 4 KiB granule; bit 0 (CnP) and
/// any other low bits are not part of the table base address.
const TTBR_BADDR_MASK: u64 = 0x0000_ffff_ffff_f000;

pub(crate) struct Walker<'vas, Prepare = fn(TableScope) -> bool> {
    pub address_space: &'vas mut AddressSpace,
    pub vaddr: VAddr,
    pub l0_ptr: *mut PageTable,
    pub l1_ptr: *mut PageTable,
    pub l2_ptr: *mut PageTable,
    pub l3_ptr: *mut PageTable,
    prepare: Prepare,
}

impl<'vas> Walker<'vas> {
    pub fn new(address_space: &'vas mut AddressSpace, vaddr: VAddr) -> Self {
        Self::with_preparer(address_space, vaddr, |_| true)
    }

    pub(crate) fn test_table_preparation() {
        // A prepared but unpublished lazy root must be released when tag
        // admission fails, without touching the active hardware root.
        let free = crate::memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let tables = crate::memory::translation::account::test_used_pages();
        let mut space = AddressSpace::try_new_user().unwrap();
        assert!(matches!(
            Walker::new(&mut space, VAddr::from(charlotte_launch::HEAP_VADDR))
                .ensure_root_with_tag(|_, _, _| false),
            Err(WalkerError::HardwareAsidExhausted)
        ));
        assert_eq!(space.hw_asid(), 0);
        assert_eq!(space.get_ttbr0(), 0);
        assert_eq!(space.table_account.pages(), 0);
        assert_eq!(crate::memory::translation::account::test_used_pages(), tables);
        drop(space);
        assert_eq!(crate::memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
        crate::logln!(
            "[table preparation] Arm tag rejection explicitly cancels unpublished root backing \
             and original table charge"
        );
        crate::memory::translation::tests::run(|space, vaddr, frame, limit| {
            let mut remaining = limit;
            let prepare = |scope| {
                assert_eq!(scope, TableScope::PrivateUser);
                if remaining == 0 {
                    return false;
                }
                remaining -= 1;
                true
            };
            Walker::with_preparer(space, vaddr, prepare).map_existing_page(
                frame,
                true,
                true,
                true,
                super::descriptor::MAIR_IDX_NORMAL,
            )
        });
    }
}

impl<'vas, Prepare: FnMut(TableScope) -> bool> Walker<'vas, Prepare> {
    fn with_preparer(
        address_space: &'vas mut AddressSpace,
        vaddr: VAddr,
        prepare: Prepare,
    ) -> Self {
        Self {
            address_space,
            vaddr,
            l0_ptr: core::ptr::null_mut(),
            l1_ptr: core::ptr::null_mut(),
            l2_ptr: core::ptr::null_mut(),
            l3_ptr: core::ptr::null_mut(),
            prepare,
        }
    }

    fn unmapped_error() -> WalkerError {
        WalkerError::Unmapped
    }

    fn already_mapped_error() -> WalkerError {
        WalkerError::AlreadyMapped
    }

    /// Whether this virtual address is translated through TTBR1 (higher half)
    /// rather than TTBR0 (lower half). Canonical AArch64 addresses in the
    /// higher half have their top bit set.
    fn is_higher_half(&self) -> bool {
        (<VAddr as Into<usize>>::into(self.vaddr) >> 63) & 1 == 1
    }

    fn prepare_table(&mut self) -> WalkerResult<PreparingTable<'_>> {
        let scope = if self.is_higher_half() {
            TableScope::SharedKernel
        } else {
            TableScope::PrivateUser
        };
        if scope == TableScope::PrivateUser && !self.address_space.owns_root {
            return Err(WalkerError::PermissionDenied);
        }
        if !(self.prepare)(scope) {
            return Err(WalkerError::PMemError(crate::memory::physical::Error::OutOfFrames));
        }
        PreparingTable::allocate(
            scope,
            (scope == TableScope::PrivateUser).then_some(&mut self.address_space.table_account),
        )
        .ok_or(WalkerError::PMemError(crate::memory::physical::Error::OutOfFrames))
    }

    /// The physical base of the root (L0) table for this address, taken from the
    /// appropriate TTBR. Returns `None` if that half has no root table yet.
    fn root_table_base(&self) -> Option<PAddr> {
        let ttbr = if self.is_higher_half() {
            self.address_space.get_ttbr1()
        } else {
            self.address_space.get_ttbr0()
        };
        let base = ttbr & TTBR_BADDR_MASK;
        if base == 0 {
            None
        } else {
            Some(PAddr::from(base))
        }
    }

    fn root_table_ptr(&self) -> WalkerResult<*mut PageTable> {
        let base = self.root_table_base().ok_or_else(Self::unmapped_error)?;
        Ok(base.into())
    }

    /// Descend one level. Returns the next-level table pointer if the descriptor
    /// at `index` is a valid table descriptor, or `Unmapped` otherwise. When
    /// `expect_block` is set a valid block descriptor is treated as success and
    /// its output frame is returned instead.
    fn walk_next_level(
        &self,
        table_ptr: *mut PageTable,
        index: usize,
    ) -> WalkerResult<*mut PageTable> {
        unsafe {
            let desc = (*table_ptr)[index];
            if !desc.is_valid() || !desc.is_table() {
                return Err(Self::unmapped_error());
            }
            Ok(desc.frame().into())
        }
    }

    /// Allocate, zero, and link a fresh next-level table under `parent[index]`.
    fn allocate_and_link_table(
        &mut self,
        parent_table_ptr: *mut PageTable,
        index: usize,
    ) -> WalkerResult<*mut PageTable> {
        let preparation = self.prepare_table()?;
        let new_table_ptr: *mut PageTable = preparation.frame().into();
        unsafe {
            let entry = &raw mut (*parent_table_ptr)[index];
            preparation.publish(|frame| {
                core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
                core::ptr::write_volatile(entry, Descriptor::new_table(frame));
            });
            Ok(new_table_ptr)
        }
    }

    fn prepare_map_walk_result(walk_result: WalkerResult<()>) -> WalkerResult<()> {
        match walk_result {
            Ok(_) => Err(Self::already_mapped_error()),
            Err(WalkerError::Unmapped) => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// Walk to the L3 page descriptor for a 4 KiB mapping.
    pub fn walk(&mut self) -> WalkerResult<()> {
        self.l0_ptr = self.root_table_ptr()?;
        self.l1_ptr = self.walk_next_level(self.l0_ptr, self.vaddr.pml4_index())?;
        self.l2_ptr = self.walk_next_level(self.l1_ptr, self.vaddr.pdpt_index())?;
        self.l3_ptr = self.walk_next_level(self.l2_ptr, self.vaddr.pd_index())?;
        unsafe {
            let desc = (*self.l3_ptr)[self.vaddr.pt_index()];
            // At L3 a valid mapping uses the page encoding (same bit pattern as
            // a table at upper levels).
            if !desc.is_valid() || !desc.is_table() {
                return Err(Self::unmapped_error());
            }
        }
        Ok(())
    }

    /// Walk to the L2 block descriptor for a 2 MiB large page mapping.
    pub fn walk_large_page(&mut self) -> WalkerResult<()> {
        self.l0_ptr = self.root_table_ptr()?;
        self.l1_ptr = self.walk_next_level(self.l0_ptr, self.vaddr.pml4_index())?;
        self.l2_ptr = self.walk_next_level(self.l1_ptr, self.vaddr.pdpt_index())?;
        self.l3_ptr = core::ptr::null_mut();
        unsafe {
            let desc = (*self.l2_ptr)[self.vaddr.pd_index()];
            if !desc.is_valid() || !desc.is_block() {
                return Err(Self::unmapped_error());
            }
        }
        Ok(())
    }

    /// Walk to the L1 block descriptor for a 1 GiB huge page mapping.
    pub fn walk_huge_page(&mut self) -> WalkerResult<()> {
        self.l0_ptr = self.root_table_ptr()?;
        self.l1_ptr = self.walk_next_level(self.l0_ptr, self.vaddr.pml4_index())?;
        self.l2_ptr = core::ptr::null_mut();
        self.l3_ptr = core::ptr::null_mut();
        unsafe {
            let desc = (*self.l1_ptr)[self.vaddr.pdpt_index()];
            if !desc.is_valid() || !desc.is_block() {
                return Err(Self::unmapped_error());
            }
        }
        Ok(())
    }

    /// Ensure the root (L0) table exists, allocating one if the relevant TTBR is
    /// empty. Kernel (higher half) mappings already have a root established by
    /// Limine; freshly created user address spaces may not.
    fn ensure_root(&mut self) -> WalkerResult<*mut PageTable> {
        self.ensure_root_with_tag(|tag, owns, ttbr| {
            AddressSpace::acquire_hw_asid(tag, owns, ttbr).is_some()
        })
    }

    fn ensure_root_with_tag(
        &mut self,
        acquire_tag: impl FnOnce(&mut super::HwAsid, &mut bool, &mut u64) -> bool,
    ) -> WalkerResult<*mut PageTable> {
        if self.root_table_base().is_none() {
            // Creating a root for an inactive address space must not install
            // that address space on the CPU doing the construction. Remember
            // whether this walker actually targets the currently selected
            // half before changing the in-memory TTBR value.
            let current = AddressSpace::get_current();
            let was_active = if self.is_higher_half() {
                current.get_ttbr1() == self.address_space.get_ttbr1()
            } else {
                current.get_ttbr0() == self.address_space.get_ttbr0()
            };
            let higher = self.is_higher_half();
            let scope = if higher {
                TableScope::SharedKernel
            } else {
                TableScope::PrivateUser
            };
            if scope == TableScope::PrivateUser && !self.address_space.owns_root {
                return Err(WalkerError::PermissionDenied);
            }
            if !(self.prepare)(scope) {
                return Err(WalkerError::PMemError(crate::memory::physical::Error::OutOfFrames));
            }
            let AddressSpace {
                table_account,
                hw_asid,
                owns_hw_asid,
                ttbr0_el1,
                ttbr1_el1,
                ..
            } = &mut *self.address_space;
            let preparation =
                PreparingTable::allocate(scope, (!higher).then_some(table_account))
                    .ok_or(WalkerError::PMemError(crate::memory::physical::Error::OutOfFrames))?;
            if !higher && !acquire_tag(hw_asid, owns_hw_asid, ttbr0_el1) {
                preparation.cancel_unpublished().map_err(WalkerError::PMemError)?;
                return Err(WalkerError::HardwareAsidExhausted);
            }
            preparation.publish(|frame| {
                let base = <PAddr as Into<u64>>::into(frame) & TTBR_BADDR_MASK;
                if higher {
                    *ttbr1_el1 = base;
                } else {
                    debug_assert_ne!(*hw_asid, 0);
                    *ttbr0_el1 = base | super::encode_hw_asid(*hw_asid);
                }
            });
            if was_active {
                self.address_space.load().expect("Failed to load new root translation table");
            }
        }
        self.l0_ptr = self.root_table_ptr().unwrap();
        Ok(self.l0_ptr)
    }

    pub fn map_page(
        &mut self,
        frame: PAddr,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
        mair_index: u64,
    ) -> WalkerResult<()> {
        self.map_page_with_attrs(frame, writable, user_accessible, no_execute, mair_index, true)
    }

    pub fn map_existing_page(
        &mut self,
        frame: PAddr,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
        mair_index: u64,
    ) -> WalkerResult<()> {
        self.map_page_with_attrs(frame, writable, user_accessible, no_execute, mair_index, false)
    }

    /// Map a single 4 KiB page of strongly-ordered device memory (MMIO). Unlike
    /// [`map_page`](Self::map_page) this uses the Device-nGnRnE memory attribute,
    /// forces execute-never, and does not zero the target (which would be an
    /// erroneous access to a device register block).
    pub fn map_mmio_page(&mut self, frame: PAddr, writable: bool) -> WalkerResult<()> {
        self.map_page_with_attrs(frame, writable, false, true, MAIR_IDX_DEVICE, false)
    }

    /// Map a single 4 KiB page of device memory (MMIO) so that it is reachable
    /// from EL0 — the userspace-driver path (architecture doc §10). Like
    /// [`map_mmio_page`](Self::map_mmio_page) it uses the Device-nGnRnE memory
    /// attribute, forces execute-never, and does not zero the register block,
    /// but it sets the user-accessible bit so a delegated driver domain can
    /// touch the device registers directly under its own page table and
    /// capability grant.
    pub fn map_user_mmio_page(&mut self, frame: PAddr, writable: bool) -> WalkerResult<()> {
        self.map_page_with_attrs(frame, writable, true, true, MAIR_IDX_DEVICE, false)
    }

    fn map_page_with_attrs(
        &mut self,
        frame: PAddr,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
        mair_index: u64,
        zero_frame: bool,
    ) -> WalkerResult<()> {
        Self::prepare_map_walk_result(self.walk())?;
        self.ensure_root()?;
        if self.l1_ptr.is_null() {
            self.l1_ptr = self.allocate_and_link_table(self.l0_ptr, self.vaddr.pml4_index())?;
        }
        if self.l2_ptr.is_null() {
            self.l2_ptr = self.allocate_and_link_table(self.l1_ptr, self.vaddr.pdpt_index())?;
        }
        if self.l3_ptr.is_null() {
            let l2e = unsafe { (*self.l2_ptr)[self.vaddr.pd_index()] };
            if l2e.is_valid() {
                return Err(Self::already_mapped_error());
            }
            self.l3_ptr = self.allocate_and_link_table(self.l2_ptr, self.vaddr.pd_index())?;
        }
        unsafe {
            if zero_frame {
                core::ptr::write_bytes(<PAddr as Into<*mut u8>>::into(frame), 0, PAGE_SIZE);
            }
            core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
            (*self.l3_ptr)[self.vaddr.pt_index()] = Descriptor::new_leaf(
                frame,
                writable,
                user_accessible,
                no_execute,
                mair_index,
                true,
            );
            if !no_execute && mair_index != MAIR_IDX_DEVICE {
                core::arch::asm!(
                    "dsb ishst",
                    "ic ialluis",
                    "dsb ish",
                    "isb",
                    options(nomem, nostack, preserves_flags),
                );
            }
        }
        tlb::inval_page(self.vaddr);
        Ok(())
    }

    pub fn unmap_page(&mut self) -> WalkerResult<PAddr> {
        self.walk()?;
        unsafe {
            let l3e = &raw mut (*self.l3_ptr)[self.vaddr.pt_index()];
            let paddr = (*l3e).frame();
            // We do not free the mapped frame; that is the caller's decision.
            (*l3e).clear();

            // Keep empty intermediate tables linked and owned until address-
            // space teardown. Hardware walkers may still name these frames;
            // neither the table lock nor clearing a leaf permits recycling
            // them. Reusing this hierarchy also avoids map/unmap table churn.
            // AddressSpace::drop reclaims the complete private tree after the
            // lifetime's threads and translations have been retired.
            tlb::inval_page(self.vaddr);
            Ok(paddr)
        }
    }

    pub fn map_large_page(
        &mut self,
        frame: PAddr,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
        mair_index: u64,
    ) -> WalkerResult<()> {
        Self::prepare_map_walk_result(self.walk_large_page())?;
        self.ensure_root()?;
        if self.l1_ptr.is_null() {
            self.l1_ptr = self.allocate_and_link_table(self.l0_ptr, self.vaddr.pml4_index())?;
        }
        if self.l2_ptr.is_null() {
            self.l2_ptr = self.allocate_and_link_table(self.l1_ptr, self.vaddr.pdpt_index())?;
        }
        unsafe {
            if (*self.l2_ptr)[self.vaddr.pd_index()].is_valid() {
                return Err(Self::already_mapped_error());
            }
            (*self.l2_ptr)[self.vaddr.pd_index()] = Descriptor::new_leaf(
                frame,
                writable,
                user_accessible,
                no_execute,
                mair_index,
                false,
            );
        }
        tlb::inval_page(self.vaddr);
        Ok(())
    }

    pub fn unmap_large_page(&mut self) -> WalkerResult<PAddr> {
        self.walk_large_page()?;
        unsafe {
            let l2e = &raw mut (*self.l2_ptr)[self.vaddr.pd_index()];
            let paddr = (*l2e).frame();
            (*l2e).clear();
            tlb::inval_page(self.vaddr);
            Ok(paddr)
        }
    }

    pub fn map_huge_page(
        &mut self,
        frame: PAddr,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
        mair_index: u64,
    ) -> WalkerResult<()> {
        Self::prepare_map_walk_result(self.walk_huge_page())?;
        self.ensure_root()?;
        if self.l1_ptr.is_null() {
            self.l1_ptr = self.allocate_and_link_table(self.l0_ptr, self.vaddr.pml4_index())?;
        }
        unsafe {
            if (*self.l1_ptr)[self.vaddr.pdpt_index()].is_valid() {
                return Err(Self::already_mapped_error());
            }
            (*self.l1_ptr)[self.vaddr.pdpt_index()] = Descriptor::new_leaf(
                frame,
                writable,
                user_accessible,
                no_execute,
                mair_index,
                false,
            );
        }
        tlb::inval_page(self.vaddr);
        Ok(())
    }

    pub fn unmap_huge_page(&mut self) -> WalkerResult<PAddr> {
        self.walk_huge_page()?;
        unsafe {
            let l1e = &raw mut (*self.l1_ptr)[self.vaddr.pdpt_index()];
            let paddr = (*l1e).frame();
            (*l1e).clear();
            tlb::inval_page(self.vaddr);
            Ok(paddr)
        }
    }

    /// Translate `self.vaddr` to a physical address by walking the tables,
    /// accounting for 4 KiB pages and 2 MiB / 1 GiB blocks.
    pub fn translate(&mut self) -> WalkerResult<PAddr> {
        match self.walk() {
            Ok(_) => {
                let frame = unsafe { (*self.l3_ptr)[self.vaddr.pt_index()].frame() };
                Ok(frame + self.vaddr.page_offset())
            }
            Err(WalkerError::Unmapped) => match self.walk_large_page() {
                Ok(_) => {
                    let frame = unsafe { (*self.l2_ptr)[self.vaddr.pd_index()].frame() };
                    let offset = self.vaddr.page_offset() + self.vaddr.pt_index() * PAGE_SIZE;
                    Ok(frame + offset)
                }
                Err(WalkerError::Unmapped) => {
                    self.walk_huge_page()?;
                    let frame = unsafe { (*self.l1_ptr)[self.vaddr.pdpt_index()].frame() };
                    let offset = self.vaddr.page_offset()
                        + self.vaddr.pt_index() * PAGE_SIZE
                        + self.vaddr.pd_index() * super::LARGE_PAGE_SIZE;
                    Ok(frame + offset)
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        }
    }

    /// Translate only an EL0-writable leaf. Charlotte does not set table-level
    /// access restrictions, so the page/block descriptor selected here holds
    /// the effective permission.
    pub fn translate_user_writable(&mut self) -> WalkerResult<PAddr> {
        let (descriptor, offset) = match self.walk() {
            Ok(()) => (unsafe { (*self.l3_ptr)[self.vaddr.pt_index()] }, self.vaddr.page_offset()),
            Err(WalkerError::Unmapped) => match self.walk_large_page() {
                Ok(()) => (
                    unsafe { (*self.l2_ptr)[self.vaddr.pd_index()] },
                    self.vaddr.page_offset() + self.vaddr.pt_index() * PAGE_SIZE,
                ),
                Err(WalkerError::Unmapped) => {
                    self.walk_huge_page()?;
                    (
                        unsafe { (*self.l1_ptr)[self.vaddr.pdpt_index()] },
                        self.vaddr.page_offset()
                            + self.vaddr.pt_index() * PAGE_SIZE
                            + self.vaddr.pd_index() * super::LARGE_PAGE_SIZE,
                    )
                }
                Err(error) => return Err(error),
            },
            Err(error) => return Err(error),
        };
        if !descriptor.is_user_accessible() || !descriptor.is_writable() {
            return Err(WalkerError::PermissionDenied);
        }
        Ok(descriptor.frame() + offset)
    }
}
