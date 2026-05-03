use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::device_manager::DeviceId;
use crate::memory::PAddr;

type PcieSegmentNum = u16;
type PcieBusNum = u8;
type PcieDeviceNum = u8;
type PcieFunctionNum = u8;

const MAX_SEGMENT_GROUPS: usize = 1 << 16; // 65536 segment groups
const MAX_DEVICES_PER_BUS: usize = 32;
const MAX_FUNCTIONS_PER_DEVICE: usize = 8;

#[derive(Debug)]
pub struct PciePath {
    segment_group: PcieSegmentNum,
    bus: PcieBusNum,
    device: PcieDeviceNum,
    function: PcieFunctionNum,
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
    pcie_segment_num: PcieSegmentNum,
    ecam_base: PAddr,
    start_bus_num: PcieBusNum,
    end_bus_num: PcieBusNum,
    buses: Vec<PcieBus>,
}

impl PcieSegment {
    pub fn new(
        pcie_segment_num: PcieSegmentNum,
        ecam_base: PAddr,
        start_bus_num: PcieBusNum,
        end_bus_num: PcieBusNum,
    ) -> Self {
        PcieSegment {
            pcie_segment_num,
            ecam_base,
            start_bus_num,
            end_bus_num,
            buses: Self::enumerate_buses(ecam_base, start_bus_num, end_bus_num),
        }
    }

    fn enumerate_buses(
        ecam_base: PAddr,
        start_bus_num: PcieBusNum,
        end_bus_num: PcieBusNum,
    ) -> Vec<PcieBus> {
        todo!(
            "Enumerate all busses in the segment by starting at the start_bus_num and recursively
        following PCIe bridges until the end_bus_num is reached. Do not probe devices or functions
        as that will be handled by PcieBus::enumerate_devices() and PcieDevice::probe()
             respectively."
        )
    }
}

#[derive(Debug)]
pub enum PcieBusTarget {
    Bridge(PcieBusNum),
    Device(PcieDevice),
}
#[derive(Debug)]
pub struct PcieBus {
    config_space_base: PAddr,
    devices: Vec<PcieBusTarget>,
}

#[derive(Debug)]
pub struct PcieDevice {
    functions: Vec<PcieFunction>,
}

#[derive(Debug)]
pub struct PcieFunction {
    id: DeviceId, /* Kernel-assigned unique identifier for this function, used for device
                   * management and driver binding */
    vendor_id: u16,
    device_id: u16,
    class_code: u32,
    mmio_base: PAddr,
}
