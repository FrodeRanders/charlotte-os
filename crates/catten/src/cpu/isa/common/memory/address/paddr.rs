use core::ops::{
    Add,
    AddAssign,
    Sub,
};

use crate::{
    cpu::isa::{
        interface::memory::address::{
            Address,
            PhysicalAddress,
            VirtualAddress,
        },
        memory::address::PADDR_MASK,
    },
    memory::HHDM_BASE,
};

#[derive(Debug, Clone, Copy)]
pub enum PAddrError {
    OutOfCpuSupportedRange(usize),
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct PAddr {
    raw: usize,
}

impl Address for PAddr {
    const MAX: Self = PAddr {
        raw: usize::MAX,
    };
    const MIN: Self = PAddr {
        raw: 0,
    };
    const NULL: Self = PAddr {
        raw: 0,
    };

    fn is_aligned_to(&self, alignment: usize) -> bool {
        self.raw.is_multiple_of(alignment)
    }

    fn is_valid(value: usize) -> bool {
        value & *PADDR_MASK == value
    }

    fn is_null(&self) -> bool {
        self.raw == 0
    }

    fn next_aligned_to(&self, alignment: usize) -> Self {
        unsafe { PAddr::from_unchecked(self.raw + (alignment - (self.raw % alignment))) }
    }

    fn prev_aligned_to(&self, alignment: usize) -> Self {
        PAddr {
            raw: if alignment.is_multiple_of(2) {
                self.raw & !(alignment - 1)
            } else {
                self.raw - (self.raw % alignment)
            },
        }
    }

    unsafe fn from_unchecked(raw: usize) -> Self {
        PAddr {
            raw,
        }
    }
}

impl PhysicalAddress for PAddr {
    unsafe fn into_hhdm_ptr<T>(self) -> *const T {
        (*HHDM_BASE).into_ptr::<T>().wrapping_byte_add(self.raw)
    }

    unsafe fn into_hhdm_mut<T>(self) -> *mut T {
        (*HHDM_BASE).into_mut::<T>().wrapping_byte_add(self.raw)
    }
}

impl<T> From<PAddr> for *const T {
    fn from(val: PAddr) -> Self {
        (*HHDM_BASE).into_ptr::<T>().wrapping_byte_add(val.raw)
    }
}

impl<T> From<PAddr> for *mut T {
    fn from(val: PAddr) -> Self {
        (*HHDM_BASE).into_mut::<T>().wrapping_byte_add(val.raw)
    }
}

impl TryFrom<usize> for PAddr {
    type Error = PAddrError;

    fn try_from(value: usize) -> Result<Self, PAddrError> {
        if value & !*PADDR_MASK != 0 {
            Err(PAddrError::OutOfCpuSupportedRange(value))
        } else {
            Ok(PAddr {
                raw: value,
            })
        }
    }
}

impl From<PAddr> for usize {
    fn from(val: PAddr) -> Self {
        val.raw
    }
}

impl From<u64> for PAddr {
    fn from(value: u64) -> Self {
        PAddr {
            raw: value as usize & *PADDR_MASK,
        }
    }
}

impl From<PAddr> for u64 {
    fn from(val: PAddr) -> Self {
        val.raw as u64
    }
}

impl Add<isize> for PAddr {
    type Output = PAddr;

    fn add(self, rhs: isize) -> Self::Output {
        PAddr::try_from(self.raw.wrapping_add(rhs as usize)).unwrap()
    }
}

impl<T> AddAssign<T> for PAddr
where
    PAddr: Add<T, Output = PAddr>,
{
    fn add_assign(&mut self, rhs: T) {
        *self = *self + rhs;
    }
}

impl Sub<isize> for PAddr {
    type Output = PAddr;

    fn sub(self, rhs: isize) -> Self::Output {
        PAddr::try_from(self.raw.wrapping_sub(rhs as usize)).unwrap()
    }
}

impl Add<usize> for PAddr {
    type Output = PAddr;

    fn add(self, rhs: usize) -> Self::Output {
        PAddr::try_from(self.raw.wrapping_add(rhs)).unwrap()
    }
}

impl Sub<usize> for PAddr {
    type Output = PAddr;

    fn sub(self, rhs: usize) -> Self::Output {
        PAddr::try_from(self.raw.wrapping_sub(rhs)).unwrap()
    }
}
