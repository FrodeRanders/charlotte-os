use core::fmt::{Debug, Display};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum VendorId {
    Intel = 0x8086,
    Amd = 0x1022,
}

impl From<u16> for VendorId {
    fn from(value: u16) -> Self {
        match value {
            0x8086 => VendorId::Intel,
            0x1022 => VendorId::Amd,
            _ => panic!("Unknown vendor ID: {:#06x}", value),
        }
    }
}

impl Display for VendorId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            VendorId::Intel => write!(f, "Intel Corporation"),
            VendorId::Amd => write!(f, "Advanced Micro Devices, Inc."),
        }
    }
}
