mod constants;

pub use constants::*;

use crate::device_manager::DeviceInterface;

/// The PCI identifier is the collection of fields that identify a PCI device and are used to
/// determine what driver should be used to operate the device. It is derived from the PCI
/// configuration space header and consists of the vendor ID, device ID, class code, subclass, and
/// programming interface fields.
///
/// The source for all IDs and codes used in this module and all its submodules is `https://pci-ids.ucw.cz/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PciIdentifier {
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
}

impl Into<DeviceInterface> for PciIdentifier {
    fn into(self) -> DeviceInterface {
        match (self.vendor_id, self.device_id, self.class_code, self.subclass, self.prog_if) {}
    }
}
