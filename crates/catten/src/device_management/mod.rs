//! # The Device Manager

use alloc::sync::Arc;
use alloc::vec::Vec;

use drivers::busses::*;
use drivers::endpoints::*;
pub mod drivers;
pub mod hw_interface;
pub mod topology;

pub struct DeviceControlPlaneTable {
    pub pcie_root_complex: Vec<pci_express::topology::PcieSegmentGroup>,
    pub uart: Vec<Arc<dyn uart::Uart>>,
}
