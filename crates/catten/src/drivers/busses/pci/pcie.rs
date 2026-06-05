use alloc::vec::Vec;
use core::ops::Deref;

use super::{Error, MAX_DEVICES_PER_BUS, MAX_FUNCTIONS_PER_DEVICE, MAX_SEGMENT_GROUPS};
use crate::cpu::isa::interface::memory::address::VirtualAddress;
use crate::device_manager::{DEVICE_TOPOLOGY, DeviceId};
use crate::drivers::busses::pci::ecam;
use crate::drivers::busses::pci::ecam::headers::Class;
use crate::drivers::busses::pci::ecam::pcie::PcieCfgSpace;
use crate::memory::{PAddr, VAddr};

pub(super) type PcieSegmentGroupNum = u16;
pub(super) type PcieBusSegmentNum = u8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct PcieDeviceNum(u8);
impl TryFrom<u8> for PcieDeviceNum {
    type Error = ();

    fn try_from(num: u8) -> Result<Self, Self::Error> {
        if num < MAX_DEVICES_PER_BUS as u8 {
            Ok(PcieDeviceNum(num))
        } else {
            Err(())
        }
    }
}
impl PcieDeviceNum {
    pub fn get_inner(self) -> u8 {
        self.0
    }
}
impl Deref for PcieDeviceNum {
    type Target = u8;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct PcieFunctionNum(u8);
impl TryFrom<u8> for PcieFunctionNum {
    type Error = ();

    fn try_from(num: u8) -> Result<Self, Self::Error> {
        if num < MAX_FUNCTIONS_PER_DEVICE as u8 {
            Ok(PcieFunctionNum(num))
        } else {
            Err(())
        }
    }
}
impl PcieFunctionNum {
    pub fn get_inner(self) -> u8 {
        self.0
    }
}
impl Deref for PcieFunctionNum {
    type Target = u8;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
pub struct PcieLocation {
    segment_group: PcieSegmentGroupNum,
    bus_segment: PcieBusSegmentNum,
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

    pub(super) fn get_cfg_space_vaddr(
        &self,
        segment_group: PcieSegmentGroupNum,
        bus_segment: PcieBusSegmentNum,
        device_num: PcieDeviceNum,
        function_num: PcieFunctionNum,
    ) -> Result<VAddr, Error> {
        let segment_group = self
            .segments
            .iter()
            .find(|sg| sg.pcie_segment_group_num == segment_group)
            .ok_or(Error::InvalidLocation)?;
        if bus_segment < segment_group.start_bus_num || bus_segment > segment_group.end_bus_num {
            return Err(Error::InvalidLocation);
        }
        let ecam_vaddr = segment_group.ecam_vaddr;
        let bus_offset = (bus_segment as usize) << 20; /* Each bus occupies 1 MiB of ECAM address space */
        let device_offset = (device_num.get_inner() as usize) << 15; /* Each device occupies 32 KiB of ECAM address space */
        let function_offset = (function_num.get_inner() as usize) << 12; /* Each function occupies 4 KiB of ECAM address space */
        Ok(ecam_vaddr + bus_offset + device_offset + function_offset)
    }
}

#[derive(Debug)]
pub struct PcieSegmentGroup {
    pcie_segment_group_num: PcieSegmentGroupNum,
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
            ecam_vaddr: ecam::map_ecam(ecam_paddr), /* TODO: Map the ECAM to a suitable region in
                                                     * the
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
    devices: [PcieDevice; MAX_DEVICES_PER_BUS],
}

impl PcieBusSegment {
    fn new(segment_group_num: PcieSegmentGroupNum, bus_num: PcieBusSegmentNum) -> Self {
        let mut devices: [PcieDevice; MAX_DEVICES_PER_BUS];
        for i in 0..MAX_DEVICES_PER_BUS {
            devices[i] = PcieDevice::new(segment_group_num, bus_num, i as u8);
        }

        PcieBusSegment {
            number: bus_num,
            devices,
        }
    }
}

pub enum PcieDevice {
    Empty,
    SingleFunc(PcieSingleFuncDevice),
    MultiFunc(PcieMultiFuncDevice),
}

impl PcieDevice {
    fn new(
        segment_group_num: PcieSegmentGroupNum,
        bus_num: PcieBusSegmentNum,
        device_num: u8,
    ) -> Self {
        let cfg_space_vaddr_result = DEVICE_TOPOLOGY.lock().get_cfg_space_vaddr(
            segment_group_num,
            bus_num,
            PcieDeviceNum(device_num),
            PcieFunctionNum(0),
        );
        if let Ok(cfg_space_vaddr) = cfg_space_vaddr_result {
            let cfg_space = unsafe { &*(cfg_space_vaddr.as_ptr() as *const ecam::PcieCfgSpace) };
            if !cfg_space.has_device_present() {
                PcieDevice::Empty
            } else if cfg_space.device_is_multifunction() {
                PcieDevice::MultiFunc(PcieMultiFuncDevice::new(
                    segment_group_num,
                    bus_num,
                    device_num,
                ))
            } else {
                PcieDevice::SingleFunc(PcieSingleFuncDevice::new(
                    segment_group_num,
                    bus_num,
                    device_num,
                ))
            }
        } else {
            PcieDevice::Empty
        }
    }
}

pub struct PcieSingleFuncDevice {
    number: PcieDeviceNum,
    function: PcieFunction,
}

impl PcieSingleFuncDevice {
    fn new(
        segment_group_num: PcieSegmentGroupNum,
        bus_num: PcieBusSegmentNum,
        device_num: u8,
    ) -> Self {
        PcieSingleFuncDevice {
            number: PcieDeviceNum(device_num),
            function: PcieFunction::new(segment_group_num, bus_num, device_num, PcieFunctionNum(0)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PcieMultiFuncDevice {
    number: PcieDeviceNum,
    functions: [PcieFunction; MAX_FUNCTIONS_PER_DEVICE],
}

impl PcieMultiFuncDevice {
    fn new(
        segment_group_num: PcieSegmentGroupNum,
        bus_num: PcieBusSegmentNum,
        device_num: u8,
    ) -> Self {
        let mut functions: [PcieFunction; MAX_FUNCTIONS_PER_DEVICE];
        for i in 0..MAX_FUNCTIONS_PER_DEVICE {
            functions[i] = PcieFunction::new(
                segment_group_num,
                bus_num,
                device_num,
                PcieFunctionNum::try_from(i as u8).unwrap(),
            );
        }

        PcieMultiFuncDevice {
            number: PcieDeviceNum(device_num),
            functions,
        }
    }
}

/* Number of 32-bit BARs */
const MAX_BAR_NUM: usize = 6;
/* Number of 64-bit BARs */
const MAX_EXT_BARS: usize = 3;

#[derive(Clone, Copy)]
union BarIoAddrs {
    pub bar32: [Option<VAddr>; MAX_BAR_NUM],
    pub bar64: [Option<VAddr>; MAX_EXT_BARS],
}

impl core::fmt::Debug for BarIoAddrs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        unsafe { write!(f, "BarIoAddrs {{ bar32: {:?}, bar64: {:?} }}", self.bar32, self.bar64) }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PcieFunction {
    Empty,
    Endpoint(PcieEndpoint),
    BridgeToPcie(PcieBusSegmentNum),
    BridgeToPciLocal(PcieBusSegmentNum),
}

impl PcieFunction {
    fn new(
        segment_group_num: PcieSegmentGroupNum,
        bus_num: PcieBusSegmentNum,
        device_num: u8,
        function_num: PcieFunctionNum,
    ) -> Self {
        let cfg_space_vaddr_result = DEVICE_TOPOLOGY.pcie.get_cfg_space_vaddr(
            segment_group_num,
            bus_num,
            PcieDeviceNum(device_num),
            function_num,
        );
        if let Ok(cfg_space_vaddr) = cfg_space_vaddr_result {
            let cfg_space = unsafe { &*(cfg_space_vaddr.into_ptr::<PcieCfgSpace>()) };
            if !cfg_space.has_device_present() {
                PcieFunction::Empty
            } else if cfg_space.device_is_bridge() {
                if cfg_space.header.common.get_class().class_code == 0x06
                    && cfg_space.header.common.get_class().subclass == 0x04
                {
                    PcieFunction::BridgeToPciLocal(cfg_space.header.bridge_header.secondary_bus_num)
                } else {
                    PcieFunction::BridgeToPcie(cfg_space.header.bridge_header.secondary_bus_num)
                }
            } else {
                PcieFunction::Endpoint(PcieEndpoint::new(
                    segment_group_num,
                    bus_num,
                    device_num,
                    function_num,
                ))
            }
        } else {
            PcieFunction::Empty
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PcieEndpoint {
    number: PcieFunctionNum,
    vendor_id: u16,
    device_id: u16,
    class: Class,
    /* Raw pointer to this function's configuration space in the kernel's address space;
     * used for reading/writing config space registers inside this PCIe bus driver ONLY
     * other drivers and the rest of the kernel should use safe functions exposed by this bus
     * driver */
    cfg_ptr: *mut PcieCfgSpace,
}

impl PcieEndpoint {
    fn new(
        segment_group_num: PcieSegmentGroupNum,
        bus_num: PcieBusSegmentNum,
        device_num: u8,
        function_num: PcieFunctionNum,
    ) -> Self {
        let cfg_space_vaddr = DEVICE_TOPOLOGY
            .pcie
            .get_cfg_space_vaddr(
                segment_group_num,
                bus_num,
                PcieDeviceNum(device_num),
                function_num,
            )
            .expect("Invalid PCIe function location");
        let cfg_space = *&cfg_space_vaddr.into_ptr::<PcieCfgSpace>();
        let vendor_id = unsafe { (*cfg_space).header.common.get_vendor_id() };
        let device_id = unsafe { (*cfg_space).header.common.get_device_id() };
        let class = unsafe { (*cfg_space).header.common.get_class() };

        PcieEndpoint {
            number: function_num,
            vendor_id,
            device_id,
            class,
            cfg_ptr: cfg_space_vaddr.into_mut(),
        }
    }
}
