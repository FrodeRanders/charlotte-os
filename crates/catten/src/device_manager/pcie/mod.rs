use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::device_manager::DeviceId;

const MAX_DEVICES_PER_BUS: usize = 32;
const MAX_FUNCTIONS_PER_DEVICE: usize = 8;

pub struct PciePath {
    domain: u16,
    bus: u8,
    device: u8,
    function: u8,
}

pub struct PcieTopology {
    domains: Vec<PcieDomain>,
}

pub struct PcieDomain {
    root_bus: PcieBus,
}

pub enum PcieBusTarget {
    Bridge(Box<PcieBus>),
    Device(PcieDevice),
}
pub struct PcieBus {
    devices: [PcieBusTarget; MAX_DEVICES_PER_BUS],
}

pub struct PcieDevice {
    functions: [PcieFunction; MAX_FUNCTIONS_PER_DEVICE],
}

pub struct PcieFunction {
    id: DeviceId,
    vendor_id: u16,
    device_id: u16,
    class_code: u32,
}
