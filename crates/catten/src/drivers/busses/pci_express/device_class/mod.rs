mod vendor_id;

use crate::device_manager::DeviceClass;
use crate::drivers::busses::pci_express::device_class::vendor_id::VendorId;

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

impl PciIdentifier {
    fn try_match_device_class_by_code(&self) -> Option<DeviceClass> {
        match self.class_code {
            // Mass Storage Controller
            0x01 => match self.subclass {
                // Serial ATA Controller
                0x06 => match self.prog_if {
                    0x01 => Some(DeviceClass::AhciStorageCtlr),
                    _ => None,
                },
                // Serial Attached SCSI Controller
                0x07 => match self.prog_if {
                    0x00 => Some(DeviceClass::ScsiSasStorageCtlr),
                    _ => None,
                },
                // Non-Volatile Memory Controller
                0x08 => match self.prog_if {
                    0x02 => Some(DeviceClass::NvmExpressStorageCtlr),
                    _ => None,
                },
                _ => None,
            },
            // Bridge
            0x06 => match self.subclass {
                0x00 => Some(DeviceClass::PcieHostBridge),
                // PCI to PCI Bridge (Also PCIe to PCIe or any combination thereof)
                0x04 => match self.prog_if {
                    0x00 => Some(DeviceClass::PciToPciBridgeNormalDecode),
                    0x01 => Some(DeviceClass::PciToPciBridgeSubtractiveDecode),
                    _ => None,
                },
                _ => None,
            },
            0x07 => match self.subclass {
                0x0 => match self.prog_if {
                    0x0 => Some(DeviceClass::Unsupported),
                    0x1..=0x6 => Some(DeviceClass::Ns16x50Uart),
                    _ => None,
                },
                _ => None,
            },
            // Base System Peripheral
            0x08 => match self.subclass {
                0x0 => match self.prog_if {
                    0x0..=0x2 => Some(DeviceClass::Unsupported),
                    0x10 => cfg_select! {
                        target_arch = "x86_64" => Some(DeviceClass::IoApic),
                        _ => Some(DeviceClass::Unsupported),
                    },
                    0x20 => cfg_select! {
                        target_arch = "x86_64" => Some(DeviceClass::IoXapic),
                        _ => Some(DeviceClass::Unsupported),
                    },
                    _ => None,
                },
                _ => None,
            },
            // Serial Bus Controller
            0xc => match self.subclass {
                0x3 => match self.prog_if {
                    0x0 | 0x10 | 0xfe => Some(DeviceClass::Unsupported),
                    0x20 => Some(DeviceClass::EhciUsbHostCtlr),
                    0x30 => Some(DeviceClass::XhciUsbHostCtlr),
                    _ => None,
                },
                0x5 => Some(DeviceClass::SmBusCtlr),
                0x7 => match self.prog_if {
                    0x0 | 0x2 => Some(DeviceClass::Unsupported),
                    0x1 => Some(DeviceClass::IpmiKcs),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    fn try_match_device_class_by_vendor_and_device_id(&self) -> Option<DeviceClass> {
        match self.vendor_id.into() {
            // Intel
            VendorId::Intel => match self.device_id {
                0x9c31 => Some(DeviceClass::IntelVtdIommu),
                _ => None,
            },
            // AMD
            VendorId::Amd => match self.device_id {
                0x1457 => Some(DeviceClass::AmdViIommu),
                _ => None,
            },
            _ => None,
        }
    }
}

impl Into<DeviceClass> for PciIdentifier {
    fn into(self) -> DeviceClass {
        if let Some(class_code_match) = self.try_match_device_class_by_code() {
            class_code_match
        } else if let Some(vendor_device_match) =
            self.try_match_device_class_by_vendor_and_device_id()
        {
            vendor_device_match
        } else {
            DeviceClass::Unknown
        }
    }
}
