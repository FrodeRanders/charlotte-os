pub mod address;
pub mod paging;
pub mod tlb;

pub use crate::cpu::isa::interface::memory::MemoryInterface;
use crate::{
    cpu::isa::aarch64::memory::address::paddr::PAddrError,
    memory::{
        linear::Error as VMemError,
        physical::Error as PMemError,
    },
};

pub struct MemoryInterfaceImpl;

impl MemoryInterface for MemoryInterfaceImpl {
    type AddressSpace = paging::AddressSpace;
    type Error = Error;
    type PAddr = address::paddr::PAddr;
    type VAddr = address::vaddr::VAddr;

    const PAGE_SIZE: usize = paging::PAGE_SIZE;
}

#[derive(Debug, Clone, Copy)]
pub enum Error {
    Unmapped,
    PermissionDenied,
    AlreadyMapped,
    NullVAddrNotAllowed,
    VAddrNotPageAligned,
    VAddrNotLargePageAligned,
    VAddrNotHugePageAligned,
    NoRequestedVAddrRegionAvailable,
    HardwareAsidExhausted,
    InvalAddrTlnRes,
    PMemError(PMemError),
    VMemError(VMemError),
}

impl From<PMemError> for Error {
    fn from(err: PMemError) -> Self {
        Error::PMemError(err)
    }
}

impl From<PAddrError> for Error {
    fn from(err: PAddrError) -> Self {
        Error::PMemError(PMemError::PAddrError(err))
    }
}

impl From<VMemError> for Error {
    fn from(err: VMemError) -> Self {
        Error::VMemError(err)
    }
}
