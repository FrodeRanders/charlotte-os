use spin::LazyLock;

use crate::drivers::busses::pci_express::pcie::{PcieLocation, PcieTopology};
use crate::environment::get_pcie_segments;

pub mod fixed_io;

pub static DEVICE_TOPOLOGY: LazyLock<DeviceTopology> = LazyLock::new(DeviceTopology::new);

pub type DeviceId = u32;

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
    Ns16x50Uart,
    I2CHostCtlr,
    SpiHostCtlr,
    AhciStorageCtlr,
    SdhciStorageCtlr,
    SerialAttachedScsi,
    // PCI Express
    PcieHostBridge,
    PciToPciBridgeNormalDecode,
    PciToPciBridgeSubtractiveDecode,
    NvmExpressStorageCtlr,
    // USB
    XhciUsbHostCtlr,
    EhciUsbHostCtlr,
    HidInputCtlr,
    CdcAcmVirtualSerial,
    CdcNcmVirtualEthernet,
    //IPMI
    IpmiKcs,
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
    // Arm platform components
    #[cfg(target_arch = "aarch64")]
    ArmPl011Uart,
    #[cfg(target_arch = "aarch64")]
    ArmGic,
    #[cfg(target_arch = "aarch64")]
    ArmSmmu,
}

pub enum DeviceLocation {
    FixedIo(fixed_io::IoRange),
    Pcie(PcieLocation),
    //Usb(usb::UsbAddress),
}

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
