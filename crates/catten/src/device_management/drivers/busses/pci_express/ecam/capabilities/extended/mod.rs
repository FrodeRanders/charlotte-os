#[allow(dead_code)]
pub mod aer;
pub mod rsbar;

use crate::klib::bitwise::mask_shift_read;

/// PCI Express Extended Capability IDs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum PcieExtCapId {
    Null = 0x0000,
    AdvancedErrorReporting = 0x0001,
    AccessControlServices = 0x000d,
    // SR-IOV
    SingleRootIoVirtualization = 0x0010,
    ResizeableBaseAddressRegisters = 0x0011,
}

/// Represents the version and offset of a PCI Express Extended Capability.
/// The lower 4 bits represent the version, and the upper 12 bits represent the offset from the
/// beginning of the PCI Express extended configuration space in the ECAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct PcieExtCapVerOffset(u16);

impl PcieExtCapVerOffset {
    pub fn new(offset: u16) -> Self {
        Self(offset)
    }

    pub fn raw(&self) -> u16 {
        self.0
    }

    pub fn get_version(&self) -> u8 {
        (self.0 & 0b1111) as u8
    }

    pub fn get_offset(&self) -> u16 {
        const OFFSET_MASK: u16 = 0xfff0;
        const OFFSET_SHIFT: u8 = 4;
        mask_shift_read(self.0, OFFSET_MASK, OFFSET_SHIFT)
    }
}

#[repr(C, packed)]
pub struct PcieExtCapabilityHeader {
    pub id: PcieExtCapId,
    pub next_ver: PcieExtCapVerOffset,
}
