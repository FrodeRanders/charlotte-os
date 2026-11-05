//! # Linear Address Map

use spin::LazyLock;

use super::VAddr;
use crate::{
    cpu::isa::memory::address::VADDR_SIG_BITS,
    klib::size::*,
};

// Range invariants adapted from upstream CharlotteOS aec5f68df26e5eeb932f6431a477260d354468fd.
// Keep this fork's active stack/MMIO/heap bases and ARM's raw TTBR1 addresses.
pub static LA_MAP: LazyLock<LinearAddressMap> =
    LazyLock::new(|| LinearAddressMap::for_width(*VADDR_SIG_BITS, cfg!(target_arch = "aarch64")));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionType {
    NullPage,
    Application,
    DirectMapping,
    KernelStackArena,
    KernelMmio,
    KernelAllocatorArena,
    KernelImage,
}

pub struct LinearAddressMap {
    null_page: LinearMemoryRegion,
    application: LinearMemoryRegion,
    direct_mapping: LinearMemoryRegion,
    kernel_stack_arena: LinearMemoryRegion,
    kernel_mmio: LinearMemoryRegion,
    kernel_allocator_arena: LinearMemoryRegion,
    kernel_image: LinearMemoryRegion,
}

impl LinearAddressMap {
    fn for_width(bits: u8, aarch64: bool) -> Self {
        let (stack, mmio, heap) = match bits {
            39 => (0xffff_ffe0_0000_0000usize, 0xffff_ffe1_0000_0000, 0xffff_ffe2_0000_0000),
            48 => (0xffff_8100_0000_0000usize, 0xffff_8200_0000_0000, 0xffff_8400_0000_0000),
            57 => (0xff80_0000_0000_0000usize, 0xff88_0000_0000_0000, 0xff90_0000_0000_0000),
            _ => panic!("Unsupported virtual address size"),
        };
        let half = 1usize << (bits - 1);
        // The shared VAddr API uses a canonical lower-half software subset.
        // ARM's TTBR1 can additionally use bit-(bits-1)-clear upper addresses,
        // including the bootloader's HHDM base. Never canonicalize those away.
        let upper = usize::MAX
            - if aarch64 {
                (1usize << bits) - 1
            } else {
                half - 1
            };
        let image = usize::MAX - gibibytes(2) + 1;
        let region = |base, length| LinearMemoryRegion {
            // All bases are checked below before the map is published.
            base: unsafe { VAddr::from_raw_unchecked(base) },
            length,
        };
        let map = Self {
            null_page: region(0, kibibytes(4)),
            application: region(kibibytes(4), half - kibibytes(4)),
            direct_mapping: region(upper, stack - upper),
            kernel_stack_arena: region(stack, mmio - stack),
            kernel_mmio: region(mmio, heap - mmio),
            kernel_allocator_arena: region(heap, image - heap),
            kernel_image: region(image, gibibytes(2)),
        };
        map.validate(half as u128, upper as u128);
        map
    }

    fn validate(&self, half: u128, upper: u128) {
        let tile = |regions: &[&LinearMemoryRegion], start: u128, end: u128| {
            let mut cursor = start;
            for region in regions {
                let base: usize = region.base.into();
                assert_eq!(base as u128, cursor, "misplaced software address region");
                assert!(region.length > 0, "empty software address region");
                cursor += region.length as u128;
            }
            assert_eq!(cursor, end, "software address regions have a gap or overlap");
        };
        tile(&[&self.null_page, &self.application], 0, half);
        tile(
            &[
                &self.direct_mapping,
                &self.kernel_stack_arena,
                &self.kernel_mmio,
                &self.kernel_allocator_arena,
                &self.kernel_image,
            ],
            upper,
            1u128 << 64,
        );
    }

    pub fn region_type(&self, addr: VAddr) -> RegionType {
        if self.null_page.contains(addr) {
            RegionType::NullPage
        } else if self.application.contains(addr) {
            RegionType::Application
        } else if self.direct_mapping.contains(addr) {
            RegionType::DirectMapping
        } else if self.kernel_stack_arena.contains(addr) {
            RegionType::KernelStackArena
        } else if self.kernel_mmio.contains(addr) {
            RegionType::KernelMmio
        } else if self.kernel_allocator_arena.contains(addr) {
            RegionType::KernelAllocatorArena
        } else if self.kernel_image.contains(addr) {
            RegionType::KernelImage
        } else {
            unreachable!("Address lies outside the configured software regions.");
        }
    }

    pub fn get_region(&self, region: RegionType) -> &LinearMemoryRegion {
        match region {
            RegionType::NullPage => &self.null_page,
            RegionType::Application => &self.application,
            RegionType::DirectMapping => &self.direct_mapping,
            RegionType::KernelStackArena => &self.kernel_stack_arena,
            RegionType::KernelMmio => &self.kernel_mmio,
            RegionType::KernelAllocatorArena => &self.kernel_allocator_arena,
            RegionType::KernelImage => &self.kernel_image,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinearMemoryRegion {
    pub base: VAddr,
    pub length: usize,
}

impl LinearMemoryRegion {
    pub fn contains(&self, addr: VAddr) -> bool {
        let base: usize = self.base.into();
        let address: usize = addr.into();
        address.checked_sub(base).is_some_and(|offset| offset < self.length)
    }
}

impl From<LinearMemoryRegion> for (VAddr, VAddr) {
    fn from(val: LinearMemoryRegion) -> Self {
        let base: usize = val.base.into();
        let end = base.checked_add(val.length).expect("exclusive region end is not representable");
        // Exclusive lower-half endpoints need not themselves be canonical.
        (val.base, unsafe { VAddr::from_raw_unchecked(end) })
    }
}

#[cfg(test)]
#[path = "address_map/tests.rs"]
mod tests;
