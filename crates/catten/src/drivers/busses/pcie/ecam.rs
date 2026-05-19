//! # PCI Express Enhanced Configuration Access Mechanism (ECAM)

use core::mem::ManuallyDrop;

use crate::memory::PAddr;
use crate::memory::physical::PhysicalAddress;

#[repr(C, packed)]
pub struct CfgCommonHeader {
    vendor_id: u16,
    device_id: u16,
    command: u16,
    status: u16,
    revision_id: u8,
    prog_if: u8,
    subclass: u8,
    class_code: u8,
    _cache_line_size: u8,
    _latency_timer: u8,
    header_type: u8,
    bist: u8,
}

impl CfgCommonHeader {
    /* Source: https://wiki.osdev.org/PCI#Configuration_Space */
    const HEADER_SINGLE_FUNC_MASK: u8 = 0b1 << 7;
    const HEADER_TYPE_MASK: u8 = 0b1;

    pub fn is_bridge(&self) -> bool {
        self.header_type & Self::HEADER_TYPE_MASK == 0b1
    }

    pub fn is_multi_function(&self) -> bool {
        self.header_type & Self::HEADER_SINGLE_FUNC_MASK == 0
    }
}

#[repr(C, packed)]
pub struct CfgBridgeHeader {
    common: CfgCommonHeader,
    bars: [u32; 2],
    primary_bus_num: u8,
    secondary_bus_num: u8,
    subordinate_bus_num: u8,
    secondary_latency_timer: u8,
    io_base: u8,
    io_limit: u8,
    secondary_status: u16,
    memory_base: u16,
    memory_limit: u16,
    prefetchable_memory_base: u16,
    prefetchable_memory_limit: u16,
    prefetchable_base_upper32: u32,
    prefetchable_limit_upper32: u32,
    io_base_upper16: u16,
    io_limit_upper16: u16,
    capabilities_ptr: u8,
    _unused0: [u8; 7],
    interrupt_line: u8,
    interrupt_pin: u8,
    _bridge_control: u16,
}

#[repr(C, packed)]
pub struct CfgEndpointHeader {
    common: CfgCommonHeader,
    bars: [u32; 6],
    _cardbus_cis_ptr: u32,
    subsystem_vendor_id: u16,
    subsystem_id: u16,
    expansion_rom_base_addr: u32,
    capabilities_ptr: u8,
    _unused0: [u8; 7],
    interrupt_line: u8,
    interrupt_pin: u8,
    _min_grant: u8,
    _max_latency: u8,
}

pub union CfgHeader {
    common: ManuallyDrop<CfgCommonHeader>, /* For determining header type before safely
                                            * accessing bridge/endpoint-specific fields */
    bridge: ManuallyDrop<CfgBridgeHeader>,
    endpoint: ManuallyDrop<CfgEndpointHeader>,
}

const PCIE_CFG_SPACE_SIZE: usize = 4096;

#[repr(C, packed)]
pub struct CfgSpace {
    header: CfgHeader,
    capability_space: [u8; PCIE_CFG_SPACE_SIZE - core::mem::size_of::<CfgHeader>()],
}

pub fn get_cfg_hhdm_ptr(ecam_base: PAddr, bus: u8, device: u8, function: u8) -> *const CfgSpace {
    let offset = ((bus as usize) << 20) | ((device as usize) << 15) | ((function as usize) << 12);
    unsafe { (ecam_base + offset).into_hhdm_ptr() }
}

pub fn get_cfg_hhdm_mut(ecam_base: PAddr, bus: u8, device: u8, function: u8) -> *mut CfgSpace {
    let offset = ((bus as usize) << 20) | ((device as usize) << 15) | ((function as usize) << 12);
    unsafe { (ecam_base + offset).into_hhdm_mut() }
}
