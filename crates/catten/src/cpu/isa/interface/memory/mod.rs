pub mod address;

use crate::cpu::isa::memory::{
    MemoryInterfaceImpl,
    address::{
        paddr::PAddr,
        vaddr::VAddr,
    },
};
pub use crate::memory::linear::{
    MemoryMapping,
    PageType,
};

pub trait MemoryInterface {
    type VAddr: address::VirtualAddress;
    type PAddr: address::PhysicalAddress;
    type Error;
    type AddressSpace: AddressSpaceInterface;

    const PAGE_SIZE: usize;
}

pub trait AddressSpaceInterface {
    const PAGE_SIZE: usize;
    const LARGE_PAGE_SIZE: usize;
    const HUGE_PAGE_SIZE: usize;

    fn get_current() -> Self;
    fn load(&self) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn find_free_region(
        &mut self,
        n_pages: usize,
        range: (VAddr, VAddr),
    ) -> Result<VAddr, <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn find_free_region_large_aligned(
        &mut self,
        n_large_pages: usize,
        range: (VAddr, VAddr),
    ) -> Result<VAddr, <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn find_free_region_huge_aligned(
        &mut self,
        n_huge_pages: usize,
        range: (VAddr, VAddr),
    ) -> Result<VAddr, <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn map_page(
        &mut self,
        mapping: MemoryMapping,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn unmap_page(
        &mut self,
        vaddr: VAddr,
    ) -> Result<PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn map_large_page(
        &mut self,
        mapping: MemoryMapping,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn unmap_large_page(
        &mut self,
        vaddr: VAddr,
    ) -> Result<PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn map_huge_page(
        &mut self,
        mapping: MemoryMapping,
    ) -> Result<(), <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn unmap_huge_page(
        &mut self,
        vaddr: VAddr,
    ) -> Result<PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn is_mapped(
        &mut self,
        vaddr: VAddr,
    ) -> Result<bool, <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn is_mapped_large_page(
        &mut self,
        vaddr: VAddr,
    ) -> Result<bool, <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn is_mapped_huge_page(
        &mut self,
        vaddr: VAddr,
    ) -> Result<bool, <MemoryInterfaceImpl as MemoryInterface>::Error>;
    fn translate_address(
        &mut self,
        vaddr: VAddr,
    ) -> Result<PAddr, <MemoryInterfaceImpl as MemoryInterface>::Error>;
}
