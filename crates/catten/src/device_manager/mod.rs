pub mod fixed_io;
pub mod pcie;
//pub mod usb;

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
    Pcie(pcie::PciePath),
    //Usb(usb::UsbPath),
}

pub struct DeviceTopology {
    fixed: fixed_io::IoMap,
    pcie:  pcie::PcieTopology,
    //usb: usb::UsbTopology,
}
