pub mod address_map;

pub use crate::cpu::isa::memory::address::{
    paddr::PAddr,
    vaddr::VAddr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidPageAttributes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageType {
    NotPresent,
    KernelCode,         //read, execute
    KernelData,         //read, write
    KernelRoData,       //read only
    UserCode,           //user, read, execute
    UserFlatImage,      //user, read, write, execute
    UserData,           //user, read, write
    UserRoData,         //user, read only
    Mmio,               //read, write, no caching
    DirectMemoryAccess, //read, write, no caching
    Framebuffer,        //read, write, write combining
}

impl PageType {
    pub fn is_user_accessible(&self) -> bool {
        matches!(
            *self,
            PageType::UserCode
                | PageType::UserFlatImage
                | PageType::UserData
                | PageType::UserRoData
        )
    }

    pub fn is_writable(&self) -> bool {
        matches!(
            *self,
            PageType::KernelData
                | PageType::UserFlatImage
                | PageType::UserData
                | PageType::Mmio
                | PageType::DirectMemoryAccess
                | PageType::Framebuffer
        )
    }

    pub fn is_no_execute(&self) -> bool {
        !matches!(*self, PageType::KernelCode | PageType::UserCode | PageType::UserFlatImage)
    }

    pub fn is_uncacheable(&self) -> bool {
        matches!(*self, PageType::Mmio | PageType::DirectMemoryAccess | PageType::Framebuffer)
    }

    pub fn should_combine_writes(&self) -> bool {
        *self == PageType::Framebuffer
    }
}
#[derive(Debug, Clone)]
pub struct MemoryMapping {
    pub vaddr: VAddr,
    pub paddr: PAddr,
    pub page_type: PageType,
}
