use alloc::vec::Vec;

use crate::device_manager::{DEVICE_TOPOLOGY, DeviceId};
use crate::drivers::busses::pci::ecam;
use crate::memory::{PAddr, VAddr};

type PcieSegmentGroupNum = u16;
type PcieBusSegmentNum = u8;
type PcieDeviceNum = u8;
type PcieFunctionNum = u8;

const MAX_SEGMENT_GROUPS: usize = 1 << 16; // 65536 segment groups
const MAX_DEVICES_PER_BUS: usize = 32;
const MAX_FUNCTIONS_PER_DEVICE: usize = 8;

#[derive(Debug)]
pub struct PcieLocation {
    segment_group: PcieSegmentGroupNum,
    bus: PcieBusSegmentNum,
    device: PcieDeviceNum,
    function: PcieFunctionNum,
}

#[derive(Debug)]
pub struct PcieTopology {
    segments: Vec<PcieSegmentGroup>,
}

impl PcieTopology {
    pub fn new(segments: Vec<PcieSegmentGroup>) -> Self {
        PcieTopology {
            segments,
        }
    }
}

#[derive(Debug)]
pub struct PcieSegmentGroup {
    pcie_segment_group_num: PcieSegmentGroupNum,
    ecam_paddr: PAddr,
    ecam_vaddr: VAddr, /* Virtual address where this segment's ECAM is mapped in the kernel's
                        * address space */
    start_bus_num: PcieBusSegmentNum,
    end_bus_num: PcieBusSegmentNum,
    topology: PcieBusSegment, /* Root bus of this segment's topology; the rest of the topology
                               * can be traversed from here */
}

impl PcieSegmentGroup {
    pub fn new(
        pcie_segment_group_num: PcieSegmentGroupNum,
        ecam_paddr: PAddr,
        start_bus_num: PcieBusSegmentNum,
        end_bus_num: PcieBusSegmentNum,
    ) -> Self {
        PcieSegmentGroup {
            pcie_segment_group_num,
            ecam_paddr,
            ecam_vaddr: VAddr::from(0usize), /* TODO: Map the ECAM to a suitable region in the
                                              * Kernel
                                              * MMIO region of the higher half */
            start_bus_num,
            end_bus_num,
            topology: PcieBusSegment::new(pcie_segment_group_num, start_bus_num),
        }
    }
}

#[derive(Debug)]
pub struct PcieBusSegment {
    number:  PcieBusSegmentNum,
    targets: [PcieBusTarget; MAX_DEVICES_PER_BUS],
}

impl PcieBusSegment {
    fn new(segment_group_num: PcieSegmentGroupNum, bus_num: PcieBusSegmentNum) -> Self {
        let mut targets = [PcieBusTarget::None; MAX_DEVICES_PER_BUS];
        for i in 0..MAX_DEVICES_PER_BUS {
            targets[i] = PcieBusTarget::new(segment_group_num, bus_num, i as u8);
        }

        PcieBusSegment {
            number: bus_num,
            targets,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PcieBusTarget {
    Bridge(PcieBusSegmentNum),
    Endpoint(PcieEndpoint),
    None,
}

impl PcieBusTarget {
    fn new(
        segment_group_num: PcieSegmentGroupNum,
        bus_num: PcieBusSegmentNum,
        device_num: PcieDeviceNum,
    ) -> Self {
        todo!()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PcieEndpoint {
    number: PcieDeviceNum,
    functions: [PcieFunction; MAX_FUNCTIONS_PER_DEVICE],
}

#[derive(Debug, Clone, Copy)]
pub struct PcieFunction {
    id: DeviceId, /* Kernel-assigned unique identifier for this function, used for device
                   * management and driver binding */
    number: PcieFunctionNum,
    vendor_id: u16,
    device_id: u16,
    class_code: u32,
    mmio_base: PAddr,
}
