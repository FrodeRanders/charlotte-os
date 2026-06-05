use super::pcie::PCIE_CFG_SPACE_SIZE;
use crate::drivers::busses::pci::ecam::headers;

const PCI_LOCAL_CFG_SPACE_SIZE: usize = 256;

#[repr(C, packed)]
/// An overlay struct representing the entire 4KB configuration space of a PCIe device in an ECAM
pub struct PcieCfgSpace {
    pub header: headers::CfgHeader,
    pub capability_space:
        [u8; PCI_LOCAL_CFG_SPACE_SIZE - core::mem::size_of::<headers::CfgHeader>()],
    pub reserved: [u8; PCIE_CFG_SPACE_SIZE - PCI_LOCAL_CFG_SPACE_SIZE],
}

impl PcieCfgSpace {
    pub fn has_device_present(&self) -> bool {
        unsafe { self.header.common.is_device_present() }
    }

    pub fn device_is_bridge(&self) -> bool {
        unsafe { self.header.common.is_bridge() }
    }

    pub fn device_is_multifunction(&self) -> bool {
        unsafe { self.header.common.is_multi_function() }
    }
}
