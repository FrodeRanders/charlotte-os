//! # The Device Manager

use core::fmt::Display;

use spin::LazyLock;

use crate::drivers::busses::pci_express::topology::{PcieLocation, PcieTopology};
use crate::environment::get_pcie_segments;

pub mod fixed_io;

pub static DEVICE_TOPOLOGY: LazyLock<DeviceTopology> = LazyLock::new(DeviceTopology::new);

/// A kernel assigned device identifier. This is not a hardware identifier and has no meaning
/// outside of the kernel.
pub type DeviceId = u32;

/// A device as seen by the kernel device manager. This is the main abstraction for devices in the
/// kernel and is what drivers and kernel subsystems interact with when dealing
/// with devices. It also provides information about the device's capabilities and configuration and
/// where in the bus topology it is located.
pub struct Device {
    pub id: DeviceId,
    pub device_type: DeviceType,
    pub device_class: DeviceInterface,
    pub location: DeviceLocation,
}

/// Device classes as seen by userspace. This is what real devices are abstracted to.
/// each corresponds to a device class trait in the `drivers` module with one or
/// more implementations provided by drivers.
pub enum DeviceType {
    PcieHostCtlr,
    UsbHostCtlr,
    Uart,
    InputCtlr,
    StorageCtlr,
    EthernetNic,
    Iommu, // Add more device types as needed
}

/// The software operating interface for a device or more properly, a device function. This is what
/// devices present to the kernel and what drivers use to interact with the device. Userspace does
/// not ever interact with this directly but can query it for debugging and informational purposes.
pub enum DeviceInterface {
    Unknown = 0,
    Unsupported = 1,
    // Generic
    Ns16550Uart,
    Ns16650Uart,
    Ns16750Uart,
    Ns16850Uart,
    Ns16950Uart,
    Ns16550MultiPortUart,
    Ns16650MultiPortUart,
    Ns16750MultiPortUart,
    Ns16850MultiPortUart,
    Ns16950MultiPortUart,
    I2CHostCtlr,
    SpiHostCtlr,
    AhciStorageCtlr,
    SdHostController,
    SerialAttachedScsi,
    // PCI Express
    PcieHostBridge,
    PciToPciBridgeNormalDecode,
    PciToPciBridgeSubtractiveDecode,
    NvmExpressStorageCtlr,
    // USB
    Usb4Router,
    XhciUsbHostCtlr,
    EhciUsbHostCtlr,
    UsbHidClass,
    CdcAcmVirtualSerial,
    CdcNcmVirtualEthernet,
    //IPMI
    IpmiKcs,
    // Graphics and Display
    AmdGpu,
    IntelGpu,
    NvidiaGpu,
    UefiGopFramebuffer,
    UsbBulkDisplayClass,
    VirtioGpu,
    // x86-64 platform components
    #[cfg(target_arch = "x86_64")]
    I8042InputCtlr,
    #[cfg(target_arch = "x86_64")]
    IoApic,
    #[cfg(target_arch = "x86_64")]
    IoXapic,
    #[cfg(target_arch = "x86_64")]
    SmBusCtlr,
    #[cfg(target_arch = "x86_64")]
    IntelVtdIommu,
    #[cfg(target_arch = "x86_64")]
    AmdViIommu,
    #[cfg(target_arch = "x86_64")]
    HighPrecisionEventTimer,
    // Arm platform components
    #[cfg(target_arch = "aarch64")]
    ArmPl011Uart,
    #[cfg(target_arch = "aarch64")]
    ArmGic,
    #[cfg(target_arch = "aarch64")]
    ArmSmmu,
}

/// This trait is implemented to provide human readable names for device interfaces generally for
/// user queries and logging.
impl Display for DeviceInterface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = match self {
            DeviceInterface::Unknown => "Unrecognized Device Interface",
            DeviceInterface::Unsupported => "Unsupported Device Interface",
            DeviceInterface::Ns16550Uart => "National Semiconductor 16550A compatible UART",
            DeviceInterface::Ns16650Uart => "National Semiconductor 16650 compatible UART",
            DeviceInterface::Ns16750Uart => "National Semiconductor 16750 compatible UART",
            DeviceInterface::Ns16850Uart => "National Semiconductor 16850 compatible UART",
            DeviceInterface::Ns16950Uart => "National Semiconductor 16950 compatible UART",
            DeviceInterface::Ns16550MultiPortUart => {
                "National Semiconductor 16550A compatible Multi-Port UART"
            }
            DeviceInterface::Ns16650MultiPortUart => {
                "National Semiconductor 16650 compatible Multi-Port UART"
            }
            DeviceInterface::Ns16750MultiPortUart => {
                "National Semiconductor 16750 compatible Multi-Port UART"
            }
            DeviceInterface::Ns16850MultiPortUart => {
                "National Semiconductor 16850 compatible Multi-Port UART"
            }
            DeviceInterface::Ns16950MultiPortUart => {
                "National Semiconductor 16950 compatible Multi-Port UART"
            }
            DeviceInterface::I2CHostCtlr => "Inter-Integrated Circuit (I2C) Host Controller",
            DeviceInterface::SpiHostCtlr => "Serial Peripheral Interface (SPI) Host Controller",
            DeviceInterface::AhciStorageCtlr => "AHCI Storage Controller",
            DeviceInterface::SdHostController => "SD Host Controller",
            DeviceInterface::SerialAttachedScsi => "Serial Attached SCSI Controller",
            DeviceInterface::PcieHostBridge => "PCI Express Host Bridge",
            DeviceInterface::PciToPciBridgeNormalDecode => {
                "PCI (Express) to PCI (Express) Bridge (Normal Decode)"
            }
            DeviceInterface::PciToPciBridgeSubtractiveDecode => {
                "PCI (Express) to PCI (Express) Bridge (Subtractive Decode)"
            }
            DeviceInterface::NvmExpressStorageCtlr => {
                "Non-Volatile Memory Express (NVMe) Storage Controller"
            }
            DeviceInterface::Usb4Router => "USB4 Router",
            DeviceInterface::XhciUsbHostCtlr => {
                "eXtensible Host Controller Interface (xHCI) compatible USB Host Controller"
            }
            DeviceInterface::EhciUsbHostCtlr => {
                "Enhanced Host Controller Interface (EHCI) compatible USB Host Controller"
            }
            DeviceInterface::UsbHidClass => "USB Human Interface Device (HID) Class Device",
            DeviceInterface::CdcAcmVirtualSerial => {
                "USB Communications Device Class (CDC) Abstract Control Model (ACM) Serial Device"
            }
            DeviceInterface::CdcNcmVirtualEthernet => {
                "USB Communications Device Class (CDC) Network Control Model (NCM) Ethernet Device"
            }
            DeviceInterface::IpmiKcs => {
                "Intelligent Platform Management Interface (IPMI) Keyboard Controller Style (KCS) \
                 Interface"
            }
            DeviceInterface::AmdGpu => {
                "Advanced Micro Devices (AMD) VGA Compatible Device, Model Unknown"
            }
            DeviceInterface::IntelGpu => "Intel Corporation VGA Compatible Device, Model Unknown",
            DeviceInterface::NvidiaGpu => "Nvidia Corporation VGA Compatible Device, Model Unknown",
            DeviceInterface::UefiGopFramebuffer => {
                "UEFI Graphics Output Protocol (GOP) Framebuffer"
            }
            DeviceInterface::UsbBulkDisplayClass => "USB Bulk Display Class Device",
            DeviceInterface::VirtioGpu => "Virtio Virtual Graphics Processing Unit (GPU)",
            #[cfg(target_arch = "x86_64")]
            DeviceInterface::I8042InputCtlr => "i8042 (PS/2) compatible Input Controller",
            #[cfg(target_arch = "x86_64")]
            DeviceInterface::IoApic => {
                "Input/Output Advanced Programmable Interrupt Controller (I/O APIC)"
            }
            #[cfg(target_arch = "x86_64")]
            DeviceInterface::IoXapic => {
                "Extended Input/Output Advanced Programmable Interrupt Controller (IOxAPIC)"
            }
            #[cfg(target_arch = "x86_64")]
            DeviceInterface::SmBusCtlr => "System Management Bus (SMBus) Controller",
            #[cfg(target_arch = "x86_64")]
            DeviceInterface::IntelVtdIommu => {
                "Intel Virtualization Technology for Directed I/O (VT-d) Input/Output Memory \
                 Management Unit (IOMMU)"
            }
            #[cfg(target_arch = "x86_64")]
            DeviceInterface::AmdViIommu => {
                "Advanced Micro Devices Virtualization (AMD-V) Input/Output Memory Management Unit \
                 (IOMMU)"
            }
            #[cfg(target_arch = "x86_64")]
            DeviceInterface::HighPrecisionEventTimer => "High Precision Event Timer",
            #[cfg(target_arch = "aarch64")]
            DeviceInterface::ArmPl011Uart => "ARM PL011 UART",
            #[cfg(target_arch = "aarch64")]
            DeviceInterface::ArmGic => "ARM Generic Interrupt Controller",
            #[cfg(target_arch = "aarch64")]
            DeviceInterface::ArmSmmu => "ARM System Memory Management Unit",
        };
        write!(f, "{}", name)
    }
}

/// A device function's location in the system's bus topology.
/// Used by drivers to access the device and configure access to it through its parent bus.
pub enum DeviceLocation {
    FixedIo(fixed_io::IoRange),
    Pcie(PcieLocation),
    //Usb(usb::UsbAddress),
}

/// This struct represents the entire topology of peripheral devices in the system as seen by the
/// kernel device manager. It is used to query for devices and their locations in the system and to
/// provide information about the system's hardware configuration to userspace and kernel
/// subsystems.
#[derive(Debug)]
pub struct DeviceTopology {
    //fixed: Option<fixed_io::IoMap>,
    pub pcie: PcieTopology,
    //usb: Option<usb::UsbTopology>,
}

impl DeviceTopology {
    pub fn new() -> Self {
        DeviceTopology {
            pcie: PcieTopology::new(get_pcie_segments()),
        }
    }
}

impl core::fmt::Display for DeviceTopology {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "PCIe:")?;
        write!(f, "{}", self.pcie)
    }
}
