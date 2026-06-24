//! # The Device Manager
pub mod drivers;
pub mod hw_interface;
pub mod topology;

pub struct DeviceTable {
    pub pcie_root_complex: Vec<drivers::busses::pci_express::PcieRootComplex>,
    pub uart: Vec<drivers::uart::Uart>,
    pub usb_hci: Vec<drivers::usb_hci::UsbHostController>,
}
