//! # PCI Express Enhanced Configuration Access Mechanism (ECAM)

use core::mem::ManuallyDrop;

use crate::memory::PAddr;
use crate::memory::physical::PhysicalAddress;

pub(super) const PCIE_VENDOR_ID_NOT_PRESENT: u16 = 0xffff;

#[repr(C, packed)]
/// The Common portion of the PCIe configuration space header; shared by both endpoint and
/// bridge devices
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
/// The configuration space header for PCIe bridge devices, which extends the common header with
/// bridge-specific fields
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

impl CfgBridgeHeader {
    pub fn get_secondary_bus_num(&self) -> u8 {
        self.secondary_bus_num
    }
}

#[repr(C, packed)]
/// The configuration space header for PCIe endpoint devices, which extends the common header with
/// endpoint-specific fields
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

/// The configuration space header for a PCIe device, which can be either a bridge or an endpoint
pub union CfgHeader {
    pub common: ManuallyDrop<CfgCommonHeader>, /* For determining header type before safely
                                                * accessing bridge/endpoint-specific fields */
    pub bridge: ManuallyDrop<CfgBridgeHeader>,
    pub endpoint: ManuallyDrop<CfgEndpointHeader>,
}

const PCIE_CFG_SPACE_SIZE: usize = 4096;

#[repr(C, packed)]
/// An overlay struct representing the entire 4KB configuration space of a PCIe device in an ECAM
pub struct CfgSpace {
    pub header: CfgHeader,
    pub capability_space: [u8; PCIE_CFG_SPACE_SIZE - core::mem::size_of::<CfgHeader>()],
}

impl CfgSpace {
    pub fn has_device_present(&self) -> bool {
        unsafe { self.header.common.vendor_id != 0xffff }
    }

    pub fn device_is_bridge(&self) -> bool {
        unsafe { self.header.common.is_bridge() }
    }

    pub fn device_is_multifunction(&self) -> bool {
        unsafe { self.header.common.is_multi_function() }
    }
}

/// Converts a given ECAM base address and PCIe device location (bus/device/function) into a const
/// pointer to the device's configuration space in the HHDM to be used for reading configuration
/// registers via volatile reads
pub fn get_cfg_hhdm_ptr(ecam_base: PAddr, bus: u8, device: u8, function: u8) -> *const CfgSpace {
    let offset = ((bus as usize) << 20) | ((device as usize) << 15) | ((function as usize) << 12);
    unsafe { (ecam_base + offset).into_hhdm_ptr() }
}

/// Converts a given ECAM base address and PCIe device location (bus/device/function) into a mutable
/// pointer to the device's configuration space in the HHDM to be used for writing configuration
/// registers via volatile writes
///
/// Note: The caller must ensure that the targeted device is not
/// currently being accessed by any other thread to avoid data races additionally PCIe writes are
/// not posted and thus the written location should be read back to ensure the write has completed
/// before any subsequent accesses to the same device are made as per the PCIe base specification.
///
/// Ideally a higher-level safe API should be implemented atop this function for accessing PCIe
/// configuration space that abstracts away these details and ensures safe access to PCIe devices.
pub fn get_cfg_hhdm_mut(ecam_base: PAddr, bus: u8, device: u8, function: u8) -> *mut CfgSpace {
    let offset = ((bus as usize) << 20) | ((device as usize) << 15) | ((function as usize) << 12);
    unsafe { (ecam_base + offset).into_hhdm_mut() }
}

pub unsafe fn cfg_read8(ecam_base: PAddr, bus: u8, device: u8, function: u8, offset: u16) -> u8 {
    let offset = ((bus as usize) << 20)
        | ((device as usize) << 15)
        | ((function as usize) << 12)
        | (offset as usize);
    let ptr = (<PAddr as Into<usize>>::into(ecam_base) + offset) as *const u8;
    unsafe { core::ptr::read_volatile(ptr) }
}

pub unsafe fn cfg_read16(ecam_base: PAddr, bus: u8, device: u8, function: u8, offset: u16) -> u16 {
    let offset = ((bus as usize) << 20)
        | ((device as usize) << 15)
        | ((function as usize) << 12)
        | (offset as usize);
    let ptr = (<PAddr as Into<usize>>::into(ecam_base) + offset) as *const u16;
    unsafe { core::ptr::read_volatile(ptr) }
}

pub unsafe fn cfg_read32(ecam_base: PAddr, bus: u8, device: u8, function: u8, offset: u16) -> u32 {
    let offset = ((bus as usize) << 20)
        | ((device as usize) << 15)
        | ((function as usize) << 12)
        | (offset as usize);
    let ptr = (<PAddr as Into<usize>>::into(ecam_base) + offset) as *const u32;
    unsafe { core::ptr::read_volatile(ptr) }
}

pub unsafe fn cfg_write8(
    ecam_base: PAddr,
    bus: u8,
    device: u8,
    function: u8,
    offset: u16,
    value: u8,
) {
    let ecam_offset = ((bus as usize) << 20)
        | ((device as usize) << 15)
        | ((function as usize) << 12)
        | (offset as usize);
    let ptr = (<PAddr as Into<usize>>::into(ecam_base) + ecam_offset) as *mut u8;
    unsafe {
        core::ptr::write_volatile(ptr, value);
        /* Ensure write has completed before
        any subsequent accesses to the same
        device are made as per the PCIe base
        specification */
        let _ = cfg_read8(ecam_base, bus, device, function, offset);
    }
}

pub unsafe fn cfg_write16(
    ecam_base: PAddr,
    bus: u8,
    device: u8,
    function: u8,
    offset: u16,
    value: u16,
) {
    let ecam_offset = ((bus as usize) << 20)
        | ((device as usize) << 15)
        | ((function as usize) << 12)
        | (offset as usize);
    let ptr = (<PAddr as Into<usize>>::into(ecam_base) + ecam_offset) as *mut u16;
    unsafe {
        core::ptr::write_volatile(ptr, value);
        /* Ensure write has completed before
        any subsequent accesses to the same
        device are made as per the PCIe base
        specification */
        let _ = cfg_read16(ecam_base, bus, device, function, offset);
    }
}

pub unsafe fn cfg_write32(
    ecam_base: PAddr,
    bus: u8,
    device: u8,
    function: u8,
    offset: u16,
    value: u32,
) {
    let ecam_offset = ((bus as usize) << 20)
        | ((device as usize) << 15)
        | ((function as usize) << 12)
        | (offset as usize);
    let ptr = (<PAddr as Into<usize>>::into(ecam_base) + ecam_offset) as *mut u32;
    unsafe {
        core::ptr::write_volatile(ptr, value);
        /* Ensure write has completed before any subsequent accesses to the same
        device are made as per the PCIe base specification */
        let _ = cfg_read32(ecam_base, bus, device, function, offset);
    }
}
