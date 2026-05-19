mod ecam;

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
        let mut buses = Vec::new();
        /* TODO: Recursively enumerate all buses in the segment. */
        buses
    }

    pub unsafe fn cfg_read8(&self, bus: u8, device: u8, function: u8, offset: u16) -> u8 {
        let offset = ((bus as usize) << 20)
            | ((device as usize) << 15)
            | ((function as usize) << 12)
            | (offset as usize);
        let ptr = (<PAddr as Into<usize>>::into(self.ecam_base) + offset) as *const u8;
        unsafe { core::ptr::read_volatile(ptr) }
    }

    pub unsafe fn cfg_read16(&self, bus: u8, device: u8, function: u8, offset: u16) -> u16 {
        let offset = ((bus as usize) << 20)
            | ((device as usize) << 15)
            | ((function as usize) << 12)
            | (offset as usize);
        let ptr = (<PAddr as Into<usize>>::into(self.ecam_base) + offset) as *const u16;
        unsafe { core::ptr::read_volatile(ptr) }
    }

    pub unsafe fn cfg_read32(&self, bus: u8, device: u8, function: u8, offset: u16) -> u32 {
        let offset = ((bus as usize) << 20)
            | ((device as usize) << 15)
            | ((function as usize) << 12)
            | (offset as usize);
        let ptr = (<PAddr as Into<usize>>::into(self.ecam_base) + offset) as *const u32;
        unsafe { core::ptr::read_volatile(ptr) }
    }

    pub unsafe fn cfg_write8(&self, bus: u8, device: u8, function: u8, offset: u16, value: u8) {
        let offset = ((bus as usize) << 20)
            | ((device as usize) << 15)
            | ((function as usize) << 12)
            | (offset as usize);
        let ptr = (<PAddr as Into<usize>>::into(self.ecam_base) + offset) as *mut u8;
        unsafe { core::ptr::write_volatile(ptr, value) }
    }

    pub unsafe fn cfg_write16(&self, bus: u8, device: u8, function: u8, offset: u16, value: u16) {
        let offset = ((bus as usize) << 20)
            | ((device as usize) << 15)
            | ((function as usize) << 12)
            | (offset as usize);
        let ptr = (<PAddr as Into<usize>>::into(self.ecam_base) + offset) as *mut u16;
        unsafe { core::ptr::write_volatile(ptr, value) }
    }

    pub unsafe fn cfg_write32(&self, bus: u8, device: u8, function: u8, offset: u16, value: u32) {
        let offset = ((bus as usize) << 20)
            | ((device as usize) << 15)
            | ((function as usize) << 12)
            | (offset as usize);
        let ptr = (<PAddr as Into<usize>>::into(self.ecam_base) + offset) as *mut u32;
        unsafe { core::ptr::write_volatile(ptr, value) }
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
