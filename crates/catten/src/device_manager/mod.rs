pub mod fixed_io;
pub mod pcie;
//pub mod usb;

pub type DeviceId = u32;

/// Device classes as seen by userspace. This is what real devices are abstracted to.
pub enum DeviceClass {
    PcieHostCtlr,
    UsbHostCtlr,
    Uart,
    // Add more device types as needed
}

pub struct DeviceNode {
    id: DeviceId,
    class: DeviceClass,
    location: DeviceLocation,
}

pub enum DeviceLocation {
    FixedIo(fixed_io::IoRange),
}

pub struct DeviceTopology {
    fixed: fixed_io::IoMap,
    pcie:  pcie::PcieTopology,
    //usb: usb::UsbTopology,
}
