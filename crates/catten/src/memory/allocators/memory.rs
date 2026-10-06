use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::{
    cpu::isa::interface::memory::AddressSpaceInterface,
    memory::{
        AddressSpace,
        KERNEL_AS,
        PHYSICAL_FRAME_ALLOCATOR,
        linear::{
            MemoryMapping,
            PageType,
            VAddr,
        },
        physical::{
            self,
            PAddr,
            PhysicalFrameAllocator,
        },
    },
};

pub(crate) mod retirement_tests;

#[derive(Debug)]
pub enum Error {
    PfaError(physical::Error),
    IsaMemoryError(crate::cpu::isa::memory::Error),
    InvalidRange,
    RetirementFailed,
}

impl From<physical::Error> for Error {
    fn from(err: physical::Error) -> Self {
        Self::PfaError(err)
    }
}

impl From<crate::cpu::isa::memory::Error> for Error {
    fn from(err: crate::cpu::isa::memory::Error) -> Self {
        Self::IsaMemoryError(err)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PageSize {
    Standard,
    Large,
    Huge,
}

impl PageSize {
    pub const fn num_bytes(self) -> usize {
        match self {
            Self::Standard => AddressSpace::PAGE_SIZE,
            Self::Large => AddressSpace::LARGE_PAGE_SIZE,
            Self::Huge => AddressSpace::HUGE_PAGE_SIZE,
        }
    }

    fn allocate(self) -> Result<PreparingKernelFrame, Error> {
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        let frame = match self {
            Self::Standard => allocator.allocate_frame(),
            Self::Large => allocator.allocate_large_frame(),
            Self::Huge => allocator.allocate_huge_frame(),
        }?;
        Ok(PreparingKernelFrame {
            frame: Some(frame),
            page_size: self,
        })
    }

    fn deallocate(
        self,
        allocator: &mut PhysicalFrameAllocator,
        frame: PAddr,
    ) -> Result<(), physical::Error> {
        match self {
            Self::Standard => allocator.deallocate_frame(frame),
            Self::Large => allocator.deallocate_large_frame(frame),
            Self::Huge => allocator.deallocate_huge_frame(frame),
        }
    }
}

/// This helper also prepares the heap before a Rust allocator exists. Keep
/// retirement metadata inline and bounded, rather than allocating during
/// rollback or requiring a Box in an error. Current boot heap preparation
/// uses at most 132 large frames; kernel stacks use 16 standard frames.
pub(crate) const KERNEL_RANGE_FRAME_CAPACITY: usize = 256;
pub(crate) static QUARANTINED_KERNEL_PAGES: AtomicUsize = AtomicUsize::new(0);

/// Owns data removed from a kernel range until completed invalidation. The
/// caller supplies this owner outside its arena/table guard. No frame address
/// escapes the receipt, and forgetting to release it quarantines rather than
/// recycling potentially reachable backing.
#[must_use]
pub struct RetiredKernelRange {
    base: VAddr,
    page_size: PageSize,
    num_pages: usize,
    frames: [Option<PAddr>; KERNEL_RANGE_FRAME_CAPACITY],
    len: usize,
    detached: bool,
    quiescent: bool,
}

impl RetiredKernelRange {
    pub fn new() -> Self {
        Self {
            base: VAddr::default(),
            page_size: PageSize::Standard,
            num_pages: 0,
            frames: [None; KERNEL_RANGE_FRAME_CAPACITY],
            len: 0,
            detached: true,
            quiescent: false,
        }
    }

    fn begin(&mut self, base: VAddr, page_size: PageSize, num_pages: usize) -> Result<(), Error> {
        let raw: usize = base.into();
        let stride = page_size.num_bytes();
        if self.len != 0
            || num_pages > KERNEL_RANGE_FRAME_CAPACITY
            || !raw.is_multiple_of(stride)
            || num_pages != 0
                && (num_pages - 1)
                    .checked_mul(stride)
                    .and_then(|offset| raw.checked_add(offset))
                    .is_none()
        {
            return Err(Error::InvalidRange);
        }
        self.base = base;
        self.page_size = page_size;
        self.num_pages = num_pages;
        self.detached = true;
        self.quiescent = false;
        Ok(())
    }

    fn retain(&mut self, frame: PAddr) {
        // begin checked capacity before any mapping mutation. Each owned
        // leaf is adopted once from the serialized hierarchy, never by ASID.
        assert!(self.len < KERNEL_RANGE_FRAME_CAPACITY);
        self.frames[self.len] = Some(frame);
        self.len += 1;
    }

    /// Finish only after arena/table and other IRQ-masking guards are gone.
    /// A mutable owner permits retry after failure without allocating an
    /// error owner before heap initialization. Drop is a quarantine fallback,
    /// never a blocking rendezvous in an unknown locking context.
    pub fn release(&mut self) -> Result<(), Error> {
        self.release_with(|base, num_pages| {
            crate::cpu::isa::memory::tlb::try_inval_range_kernel(base, num_pages).is_ok()
        })
    }

    fn release_with(&mut self, invalidate: impl FnOnce(VAddr, usize) -> bool) -> Result<(), Error> {
        if self.len == 0 {
            return Ok(());
        }
        if !self.detached {
            return Err(Error::RetirementFailed);
        }
        if !self.quiescent {
            let pages = self.num_pages * (self.page_size.num_bytes() / AddressSpace::PAGE_SIZE);
            if !invalidate(self.base, pages) {
                return Err(Error::RetirementFailed);
            }
            self.quiescent = true;
        }
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        for frame in &mut self.frames[..self.len] {
            if let Some(address) = *frame {
                self.page_size.deallocate(&mut allocator, address)?;
                *frame = None;
            }
        }
        self.len = 0;
        Ok(())
    }
}

impl Drop for RetiredKernelRange {
    fn drop(&mut self) {
        let remaining = self.frames[..self.len].iter().filter(|frame| frame.is_some()).count();
        if remaining != 0 {
            let pages = remaining * (self.page_size.num_bytes() / AddressSpace::PAGE_SIZE);
            QUARANTINED_KERNEL_PAGES.fetch_add(pages, Ordering::Relaxed);
            crate::early_logln!(
                "[memory] quarantined {} kernel backing page(s) at {:?}; detached={} quiescent={}",
                pages,
                self.base,
                self.detached,
                self.quiescent
            );
            // Physical ownership remains marked unavailable in the allocator.
            // Do not follow stale mappings or publish speculative free space.
        }
    }
}

struct PreparingKernelFrame {
    frame: Option<PAddr>,
    page_size: PageSize,
}

impl PreparingKernelFrame {
    fn frame(&self) -> PAddr {
        self.frame.unwrap()
    }

    fn install(mut self) {
        self.frame.take();
    }

    fn retire(mut self, retirement: &mut RetiredKernelRange) {
        retirement.retain(self.frame.take().unwrap());
    }
}

impl Drop for PreparingKernelFrame {
    fn drop(&mut self) {
        let Some(frame) = self.frame.take() else {
            return;
        };
        // Release the allocator guard before invoking diagnostics.
        let failed =
            self.page_size.deallocate(&mut PHYSICAL_FRAME_ALLOCATOR.lock(), frame).is_err();
        if failed {
            QUARANTINED_KERNEL_PAGES
                .fetch_add(self.page_size.num_bytes() / AddressSpace::PAGE_SIZE, Ordering::Relaxed);
            crate::early_logln!("[memory] quarantined unpublished kernel frame {:?}", frame);
        }
    }
}

pub fn try_allocate_and_map_range(
    base: VAddr,
    page_size: PageSize,
    num_pages: usize,
    retirement: &mut RetiredKernelRange,
) -> Result<(), Error> {
    let mapping_func = match page_size {
        PageSize::Standard => AddressSpace::map_page,
        PageSize::Large => AddressSpace::map_large_page,
        PageSize::Huge => AddressSpace::map_huge_page,
    };
    allocate_and_map_with(
        base,
        page_size,
        num_pages,
        retirement,
        |size, _| size.allocate(),
        |space, mapping, _| mapping_func(space, mapping),
    )
}

fn allocate_and_map_with(
    base: VAddr,
    page_size: PageSize,
    num_pages: usize,
    retirement: &mut RetiredKernelRange,
    mut allocate: impl FnMut(PageSize, usize) -> Result<PreparingKernelFrame, Error>,
    mut map: impl FnMut(
        &mut AddressSpace,
        MemoryMapping,
        usize,
    ) -> Result<(), crate::cpu::isa::memory::Error>,
) -> Result<(), Error> {
    // Like the architecture mappers, an adapter returning Err must not have
    // published its leaf. Intermediate tables may remain owned and cached.
    retirement.begin(base, page_size, num_pages)?;
    let mut kas = KERNEL_AS.lock();
    for index in 0..num_pages {
        let result = allocate(page_size, index).and_then(|frame| {
            let result = map(
                &mut kas,
                MemoryMapping {
                    vaddr: base + index * page_size.num_bytes(),
                    paddr: frame.frame(),
                    page_type: PageType::KernelData,
                },
                index,
            );
            if let Err(error) = result {
                // Carry the unpublished frame in the same post-guard receipt
                // as the installed prefix. Rollback must report its physical
                // release failure to the original admission owner too.
                frame.retire(retirement);
                return Err(error.into());
            }
            frame.install();
            Ok(())
        });
        if let Err(error) = result {
            // Only this operation's successfully installed prefix is owned.
            // In particular, AlreadyMapped must not steal the failing leaf.
            detach_locked(&mut kas, base, page_size, index, retirement)?;
            return Err(error);
        }
    }
    Ok(())
}

impl Default for RetiredKernelRange {
    fn default() -> Self {
        Self::new()
    }
}

/// Caller retains arena serialization; this takes only the page-table guard.
/// All physical release is deferred to the supplied receipt after guards drop.
pub fn retire_kernel_range(
    base: VAddr,
    page_size: PageSize,
    num_pages: usize,
    retirement: &mut RetiredKernelRange,
) -> Result<(), Error> {
    retirement.begin(base, page_size, num_pages)?;
    detach_locked(&mut KERNEL_AS.lock(), base, page_size, num_pages, retirement)
}

fn detach_locked(
    kas: &mut AddressSpace,
    base: VAddr,
    page_size: PageSize,
    num_pages: usize,
    retirement: &mut RetiredKernelRange,
) -> Result<(), Error> {
    retirement.num_pages = num_pages;
    let unmap = match page_size {
        PageSize::Standard => AddressSpace::unmap_page,
        PageSize::Large => AddressSpace::unmap_large_page,
        PageSize::Huge => AddressSpace::unmap_huge_page,
    };
    for index in 0..num_pages {
        let vaddr = base + index * page_size.num_bytes();
        match kas.translate_address(vaddr) {
            Ok(frame) => {
                retirement.retain(frame);
                if !matches!(unmap(kas, vaddr), Ok(unmapped) if unmapped == frame) {
                    retirement.detached = false;
                }
            }
            Err(_) => retirement.detached = false,
        }
    }
    if retirement.detached {
        Ok(())
    } else {
        Err(Error::RetirementFailed)
    }
}
