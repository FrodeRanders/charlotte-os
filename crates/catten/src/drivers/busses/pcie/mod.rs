mod ecam;

use alloc::collections::vec_deque::VecDeque;
use alloc::vec::Vec;

use crate::device_manager::DeviceId;
use crate::drivers::busses::pcie::ecam::get_cfg_hhdm_ptr;
use crate::memory::PAddr;

type PcieSegmentNum = u16;
type PcieBusNum = u8;
type PcieDeviceNum = u8;
type PcieFunctionNum = u8;

const MAX_SEGMENT_GROUPS: usize = 1 << 16; // 65536 segment groups
const MAX_DEVICES_PER_BUS: usize = 32;
const MAX_FUNCTIONS_PER_DEVICE: usize = 8;

#[derive(Debug)]
pub struct PcieLocation {
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
    topology: PcieBus, /* Root bus of this segment's topology; the rest of the topology can be
                        * traversed from here */
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
            topology: PcieBus::new(pcie_segment_num, start_bus_num),
        }
    }
}

#[derive(Debug)]
pub struct PcieBus {
    number:  PcieBusNum,
    targets: [PcieBusTarget; MAX_DEVICES_PER_BUS],
}

impl PcieBus {
    fn new(segment_num: PcieSegmentNum, bus_num: PcieBusNum) -> Self {
        let mut targets = [PcieBusTarget::None; MAX_DEVICES_PER_BUS];
        for i in 0..MAX_DEVICES_PER_BUS {
            targets[i] = PcieBusTarget::new(segment_num, bus_num, i as u8);
        }

        PcieBus {
            number: bus_num,
            targets,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PcieBusTarget {
    Bridge(PcieBusNum),
    Endpoint(PcieEndpoint),
    None,
}

impl PcieBusTarget {
    fn new(segment_num: PcieSegmentNum, bus_num: PcieBusNum, device_num: PcieDeviceNum) -> Self {
        let cfg_ptr = ecam::get_cfg_hhdm_ptr(segment.ecam_base, bus_num, device_num, 0);
        let vendor_id = unsafe { core::ptr::read_volatile(&(*cfg_ptr).header.common.vendor_id) };
        if vendor_id == ecam::PCIE_VENDOR_ID_NOT_PRESENT {
            PcieBusTarget::None
        } else {
            if unsafe { (*cfg_ptr).device_is_bridge() } {
                let secondary_bus_num = unsafe { (*cfg_ptr).header.bridge.get_secondary_bus_num() };
                PcieBusTarget::Bridge(secondary_bus_num)
            } else {
                PcieBusTarget::Endpoint(PcieEndpoint::new(segment, bus_num, device_num))
            }
        }
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
