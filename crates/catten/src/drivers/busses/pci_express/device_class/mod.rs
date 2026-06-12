mod constants;

use core::fmt::Display;

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

impl Display for PciIdentifier {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "0x{:04x}:0x{:04x} (class 0x{:02x}, subclass 0x{:02x}, prog IF 0x{:02x}) => {}",
            self.vendor_id,
            self.device_id,
            self.class_code,
            self.subclass,
            self.prog_if,
            Into::<DeviceInterface>::into(*self)
        )
    }
}

impl Into<DeviceInterface> for PciIdentifier {
    fn into(self) -> DeviceInterface {
        match (self.vendor_id, self.device_id, (self.class_code, self.subclass, self.prog_if)) {
            /* AMD */
            (vendor_id::AMD, _, device_class::VGA_COMPATIBLE) => DeviceInterface::AmdGpu,
            #[cfg(target_arch = "x86_64")]
            (vendor_id::AMD, _, device_class::IOMMU) => DeviceInterface::AmdViIommu,

            /* ARM */
            #[cfg(target_arch = "aarch64")]
            (vendor_id::ARM, _, device_class::VGA_COMPATIBLE) => DeviceInterface::ArmGpu,

            /* Intel */
            (vendor_id::INTEL, _, device_class::VGA_COMPATIBLE) => DeviceInterface::IntelGpu,

            /* Nvidia */
            (vendor_id::NVIDIA, _, device_class::VGA_COMPATIBLE) => DeviceInterface::NvidiaGpu,

            /* virtio */
            // This is technically virtio-vga but we don't use the legacy VGA interface just the one
            // it shares with virtio-gpu so we map it to virtio-gpu.
            #[cfg(feature = "virtio_gpu")]
            (vendor_id::REDHAT, _, device_class::VGA_COMPATIBLE) => DeviceInterface::VirtioGpu,
            #[cfg(feature = "virtio_gpu")]
            (vendor_id::REDHAT, _, device_class::OTHER_DISPLAY_CONTROLLER) => {
                DeviceInterface::VirtioGpu
            }

            /* Generic Device Class Interfaces

            These purposely bind less tightly than vendor specific matches since there could be devices that use the
            same class codes but require device specific drivers or special handling in the same driver.
            */
            (_, _, device_class::HOST_BRIDGE) => DeviceInterface::PcieHostBridge,
            (_, _, device_class::PCI_TO_PCI_BRIDGE) => DeviceInterface::PciToPciBridgeNormalDecode,
            (_, _, device_class::PCI_TO_PCI_BRIDGE_SUB_DEC) => {
                DeviceInterface::PciToPciBridgeSubtractiveDecode
            }
            (_, _, device_class::NS16550) => DeviceInterface::Ns16550Uart,
            (_, _, device_class::NS16650) => DeviceInterface::Ns16650Uart,
            (_, _, device_class::NS16750) => DeviceInterface::Ns16750Uart,
            (_, _, device_class::NS16850) => DeviceInterface::Ns16850Uart,
            (_, _, device_class::NS16950) => DeviceInterface::Ns16950Uart,
            (_, _, device_class::NS16550_MULTI_PORT) => DeviceInterface::Ns16550MultiPortUart,
            (_, _, device_class::NS16650_MULTI_PORT) => DeviceInterface::Ns16650MultiPortUart,
            (_, _, device_class::NS16750_MULTI_PORT) => DeviceInterface::Ns16750MultiPortUart,
            (_, _, device_class::NS16850_MULTI_PORT) => DeviceInterface::Ns16850MultiPortUart,
            (_, _, device_class::NS16950_MULTI_PORT) => DeviceInterface::Ns16950MultiPortUart,
            #[cfg(target_arch = "x86_64")]
            (_, _, device_class::IOAPIC) => DeviceInterface::IoApic,
            #[cfg(target_arch = "x86_64")]
            (_, _, device_class::IOXAPIC) => DeviceInterface::IoXapic,
            #[cfg(target_arch = "x86_64")]
            (_, _, device_class::HPET) => DeviceInterface::HighPrecisionEventTimer,
            (_, _, device_class::SD_HOST_CONTROLLER) => DeviceInterface::SdHostController,
            (_, _, device_class::USB_EHCI) => DeviceInterface::EhciUsbHostCtlr,
            (_, _, device_class::USB_XHCI) => DeviceInterface::XhciUsbHostCtlr,
            (_, _, device_class::USB4_ROUTER) => DeviceInterface::Usb4Router,
            (_, _, device_class::SMBUS_CONTROLLER) => DeviceInterface::SmBusCtlr,
            (_, _, device_class::IPMI_KCS) => DeviceInterface::IpmiKcs,

            /* Unrecognized Devices */
            (_, _, (_, _, _)) => DeviceInterface::Unknown,
        }
    }
}
