use spin::Lazy;

use crate::drivers::busses::pcie;
use crate::environment::get_pcie_segments;

pub mod fixed_io;

pub static DEVICE_TOPOLOGY: Lazy<DeviceTopology> = Lazy::new(DeviceTopology::new);

pub type DeviceId = u32;

/// Device classes as seen by userspace. This is what real devices are abstracted to.
/// each corresponds to a device class trait in the `drivers` module with one or
/// more implementations provided by drivers.
pub enum DeviceClass {
    PcieHostCtlr,
    UsbHostCtlr,
    Uart,
    InputCtlr,
    StorageCtlr,
    EthernetNic,
    // Add more device types as needed
}

pub struct DeviceNode {
    id: DeviceId,
    class: DeviceClass,
    location: DeviceLocation,
}

pub enum DeviceLocation {
    FixedIo(fixed_io::IoRange),
    Pcie(pcie::PcieAddress),
    //Usb(usb::UsbAddress),
}

#[derive(Debug)]
pub struct DeviceTopology {
    //fixed: Option<fixed_io::IoMap>,
    pcie: pcie::PcieTopology,
    //usb: Option<usb::UsbTopology>,
}

impl DeviceTopology {
    pub fn new() -> Self {
        DeviceTopology {
            pcie: pcie::PcieTopology::new(get_pcie_segments()),
        }
    }
}
