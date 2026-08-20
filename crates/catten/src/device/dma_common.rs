//! Shared DMA-domain types for the x86_64 IOMMU drivers (VT-d and AMD-Vi).
//!
//! Both drivers expose the same interface to the device-capability layer, so
//! they share one error and direction type rather than duplicating them.

#[derive(Debug)]
pub enum Error {
    Unsupported,
    InvalidStream,
    StreamInUse,
    InvalidDirection,
    Memory,
    OutOfIova,
    MapFailed,
    UnknownDomain,
    UnknownMapping,
    HardwareTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Direction(u32);

impl Direction {
    pub const DEVICE_READ: Self = Self(1);
    pub const DEVICE_WRITE: Self = Self(2);

    pub fn from_bits(bits: u32) -> Result<Self, Error> {
        if bits != 0 && bits & !3 == 0 {
            Ok(Self(bits))
        } else {
            Err(Error::InvalidDirection)
        }
    }

    pub(crate) fn device_writes(self) -> bool {
        self.0 & Self::DEVICE_WRITE.0 != 0
    }

    pub(crate) fn device_reads(self) -> bool {
        self.0 & Self::DEVICE_READ.0 != 0
    }
}
