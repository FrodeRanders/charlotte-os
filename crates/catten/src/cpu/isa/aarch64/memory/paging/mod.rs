//! # AArch64 Paging (VMSAv8-64, 4 KiB granule)

pub mod descriptor;
pub mod walker;

use alloc::vec::Vec;
use core::{
    arch::asm,
    ptr::NonNull,
    sync::atomic::AtomicUsize,
};

use descriptor::Descriptor;

use super::MemoryInterfaceImpl;
use crate::{
    cpu::{
        isa::{
            aarch64::memory::address::{
                paddr::PAddr,
                vaddr::VAddr,
            },
            interface::memory::{
                AddressSpaceInterface,
                MemoryInterface,
                MemoryMapping,
                address::{
                    Address,
                    VirtualAddress,
                },
            },
        },
        scheduler::system_scheduler::MAX_TRACKED_LPS,
    },
    klib::size::{
        gibibytes,
        kibibytes,
        mebibytes,
    },
    memory::{
        KERNEL_ASID,
        LazyLock,
        Mutex,
    },
};

/// Hardware Address Space Identifier. On AArch64 the ASID is held in the top
/// bits of `TTBR0_EL1`/`TTBR1_EL1` and tags TLB entries. It is 8 or 16 bits
/// wide depending on `TCR_EL1.AS`; we model the full 16-bit width and mask as
/// required.
pub type HwAsid = u16;

const TTBR_BADDR_MASK: u64 = 0x0000_ffff_ffff_f000;

/// Hardware ASID width selected by TCR_EL1.AS: 8 bits when clear, 16 bits
/// when set. In the 8-bit format the tag occupies TTBR bits 63:56.
static HW_ASID_BITS: LazyLock<u8> = LazyLock::new(|| {
    let tcr: u64;
    unsafe {
        asm!("mrs {tcr}, tcr_el1", tcr = out(reg) tcr, options(nomem, nostack, preserves_flags));
    }
    assert_eq!(
        tcr & (1 << 22),
        0,
        "CharlotteOS requires TCR_EL1.A1=0 so TTBR0 selects the user ASID"
    );
    if tcr & (1 << 36) != 0 {
        16
    } else {
        8
    }
});

fn hw_asid_shift() -> u32 {
    if *HW_ASID_BITS == 16 {
        48
    } else {
        56
    }
}

pub(crate) fn encode_hw_asid(asid: HwAsid) -> u64 {
    (asid as u64) << hw_asid_shift()
}

struct HwAsidAllocator {
    next: u32,
    limit: u32,
    reserved_kernel: HwAsid,
    free: Vec<HwAsid>,
}

impl HwAsidAllocator {
    fn new() -> Self {
        let ttbr0: u64;
        unsafe {
            asm!("mrs {ttbr0}, ttbr0_el1", ttbr0 = out(reg) ttbr0, options(nomem, nostack, preserves_flags));
        }
        Self {
            next: 1,
            limit: 1u32 << *HW_ASID_BITS,
            reserved_kernel: ((ttbr0 >> hw_asid_shift()) & 0xffff) as HwAsid,
            free: Vec::new(),
        }
    }

    fn allocate(&mut self) -> Option<HwAsid> {
        if let Some(asid) = self.free.pop() {
            return Some(asid);
        }
        if self.next == self.reserved_kernel as u32 {
            self.next += 1;
        }
        if self.next >= self.limit {
            return None;
        }
        let asid = self.next as HwAsid;
        self.next += 1;
        Some(asid)
    }

    fn release(&mut self, asid: HwAsid) {
        debug_assert_ne!(asid, 0);
        self.free.push(asid);
    }
}

static HW_ASID_ALLOCATOR: LazyLock<Mutex<HwAsidAllocator>> =
    LazyLock::new(|| Mutex::new(HwAsidAllocator::new()));

/// The logical [`AddressSpaceId`](crate::memory::AddressSpaceId) of the thread
/// currently executing on each logical processor, maintained by the context
/// switch and read by synchronous EL0 exception paths (e.g. the SVC handler) to
/// attribute the caller's syscalls.
///
/// This holds CharlotteOS's *logical* address-space id — an index into
/// [`ADDRESS_SPACE_TABLE`](crate::memory::ADDRESS_SPACE_TABLE) — not the
/// hardware [`HwAsid`](crate::cpu::isa::aarch64::memory::paging::walker::HwAsid)
/// tag encoded into TTBR0. The caller ASID must not be reconstructed from
/// `TTBR0_EL1`: some hypervisors (notably Apple's Hypervisor.framework) do not
/// preserve the ASID bits when the guest reads the register while running at
/// EL0, so `mrs ttbr0_el1` can return the base address with the tag stripped.
/// Tracking the logical id on the software side during `switch_ctx` keeps the
/// exception-path lookup reliable across TCG and HVF.
pub static CURRENT_LOGICAL_ASID: [AtomicUsize; MAX_TRACKED_LPS] =
    [const { AtomicUsize::new(KERNEL_ASID) }; MAX_TRACKED_LPS];

pub(crate) fn self_test_hw_asid_allocator() {
    let mut allocator = HwAsidAllocator {
        next: 1,
        limit: 4,
        reserved_kernel: 2,
        free: Vec::new(),
    };
    assert_eq!(allocator.allocate(), Some(1));
    assert_eq!(allocator.allocate(), Some(3));
    assert_eq!(allocator.allocate(), None, "exhaustion must be reported without panicking");
    allocator.release(1);
    assert_eq!(allocator.allocate(), Some(1), "released tags must remain recyclable");
}

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
#[derive(Debug)]
pub struct AddressSpace {
    ttbr0_el1: u64,
    ttbr1_el1: u64,
    hw_asid: HwAsid,
    owns_hw_asid: bool,
}

impl AddressSpace {
    /// Construct an inactive user address space sharing only the current
    /// kernel (TTBR1) mappings. The lower-half root and hardware ASID are
    /// assigned lazily, before the first user mapping or table registration.
    pub fn new_user() -> Self {
        let current = Self::get_current();
        Self {
            ttbr0_el1: 0,
            ttbr1_el1: current.ttbr1_el1,
            hw_asid: 0,
            owns_hw_asid: false,
        }
    }

    pub fn get_ttbr0(&self) -> u64 {
        self.ttbr0_el1
    }

    pub fn get_ttbr1(&self) -> u64 {
        self.ttbr1_el1
    }

    pub(super) fn set_ttbr1(&mut self, ttbr1: u64) {
        self.ttbr1_el1 = ttbr1;
    }

    pub fn hw_asid(&self) -> HwAsid {
        self.hw_asid
    }

    pub(crate) fn ensure_hw_asid(&mut self) -> Option<HwAsid> {
        if self.hw_asid == 0 {
            self.hw_asid = HW_ASID_ALLOCATOR.lock().allocate()?;
            self.owns_hw_asid = true;
            self.ttbr0_el1 = (self.ttbr0_el1 & TTBR_BADDR_MASK) | encode_hw_asid(self.hw_asid);
        }
        Some(self.hw_asid)
    }

    /// Install a lower-half root without disturbing this address space's TLB
    /// identity. Fresh user spaces acquire a nonzero tag on first mapping.
    pub(super) fn install_ttbr0_base(&mut self, base: u64) {
        debug_assert_ne!(self.hw_asid, 0);
        self.ttbr0_el1 = (base & TTBR_BADDR_MASK) | encode_hw_asid(self.hw_asid);
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
            hw_asid: ((ttbr0_el1 >> hw_asid_shift()) & 0xffff) as HwAsid,
            owns_hw_asid: false,
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

    fn translate_user_writable_address(
        &mut self,
        vaddr: VAddr,
    ) -> Result<PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error> {
        let mut walker = walker::Walker::new(self, vaddr);
        walker.translate_user_writable()
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        if self.owns_hw_asid && self.hw_asid != 0 {
            // A tag cannot be reused until all cores have discarded entries
            // belonging to its previous page-table lifetime.
            super::tlb::inval_hardware_asid(self.hw_asid);
            HW_ASID_ALLOCATOR.lock().release(self.hw_asid);
            self.owns_hw_asid = false;
        }
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
