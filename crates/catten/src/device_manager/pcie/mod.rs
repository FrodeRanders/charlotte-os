use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::device_manager::DeviceId;
use crate::memory::PAddr;

const MAX_SEGMENT_GROUPS: usize = 1 << 16; // 65536 segment groups
const MAX_DEVICES_PER_BUS: usize = 32;
const MAX_FUNCTIONS_PER_DEVICE: usize = 8;

#[derive(Debug)]
pub struct PciePath {
    segment_group: u16,
    bus: u8,
    device: u8,
    function: u8,
}

#[derive(Debug)]
pub struct PcieTopology {
    segments: Vec<PcieSegment>,
}

impl PcieTopology {
    pub fn new(segments: Vec<PcieSegment>) -> Self {
        PcieTopology {
            segments,
        }
    }
}

#[derive(Debug)]
pub struct PcieSegment {
    pcie_segment_num: u16,
    ecam_base: PAddr,
    start_bus_num: u8,
    end_bus_num: u8,
}

impl PcieSegment {
    pub fn new(
        pcie_segment_num: u16,
        ecam_base: PAddr,
        start_bus_num: u8,
        end_bus_num: u8,
    ) -> Self {
        PcieSegment {
            pcie_segment_num,
            ecam_base,
            start_bus_num,
            end_bus_num,
        }
    }
}

pub enum PcieBusTarget {
    Bridge(Box<PcieBus>),
    Device(PcieDevice),
}
pub struct PcieBus {
    config_space_base: PAddr,
    devices: Vec<PcieBusTarget>,
}

pub struct PcieDevice {
    functions: Vec<PcieFunction>,
}

pub struct PcieFunction {
    id: DeviceId,
    vendor_id: u16,
    device_id: u16,
    class_code: u32,
    mmio_base: PAddr,
}
