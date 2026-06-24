//! # Device Drivers

use alloc::sync::{Arc, Weak};

use crate::device_management::Device;
use crate::device_management::drivers::busses::pci_express::topology::PcieSegmentGroup;
use crate::device_management::hw_interface::HwDeviceIfce;

pub mod busses;
pub mod ethernet;
pub mod input_ctlr;
pub mod iommu;
pub mod persistent_storage;
pub mod uart;
pub mod usb_hci;

pub enum Error {
    DeviceNotRecognized,
    InitializationFailed,
    DeinitializationFailed,
    DeviceAlreadyBoundToDriver,
}

/// Device classes as abstracted by drivers
///
/// Each corresponds to a device type trait in the subordinate modules of this one with one or
/// more implementations provided by individual specific drivers.
pub enum DeviceDriver {
    PcieHostController(Arc<PcieDriver>),
    UsbHostController(Arc<usb_hci::UsbHostControllerDriver>),
    Uart(Arc<uart::UartDriver>),
    InputController(Arc<input_ctlr::InputControllerDriver>),
    StorageController(Arc<persistent_storage::StorageControllerDriver>),
    EthernetNic(Arc<ethernet::NicDriver>),
    Iommu(Arc<iommu::IommuDriver>),
}
