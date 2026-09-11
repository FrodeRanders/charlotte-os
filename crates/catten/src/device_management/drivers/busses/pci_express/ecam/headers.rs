#![allow(dead_code)]
#[allow(dead_code)]
use core::mem::ManuallyDrop;

use crate::device_management::drivers::busses::pci_express::{
    device_class::PciIdentifier,
    ecam::capabilities::standard::PciCapabilityOffset,
};

/// Volatile little-endian byte reads from an MMIO configuration-space overlay.
///
/// The `repr(C, packed)` overlays are only a layout description: fields must
/// not be read through ordinary struct field accesses, which the compiler may
/// cache, merge, or reorder against a device's changing registers. Reading
/// byte at a time also avoids unaligned typed loads.
pub(crate) unsafe fn read_u8(base: *const u8, offset: usize) -> u8 {
    unsafe { core::ptr::read_volatile(base.add(offset)) }
}

pub(crate) unsafe fn read_u16(base: *const u8, offset: usize) -> u16 {
    u16::from_le_bytes([unsafe { read_u8(base, offset) }, unsafe { read_u8(base, offset + 1) }])
}

pub(crate) unsafe fn read_u32(base: *const u8, offset: usize) -> u32 {
    u32::from_le_bytes([
        unsafe { read_u8(base, offset) },
        unsafe { read_u8(base, offset + 1) },
        unsafe { read_u8(base, offset + 2) },
        unsafe { read_u8(base, offset + 3) },
    ])
}

#[repr(C, packed)]
/// The Common portion of the PCIe configuration space header; shared by both endpoint and bridge
/// devices
pub struct CfgCommonHeader {
    vendor_id: u16,
    device_id: u16,
    command: u16,
    status: u16,
    revision_id: u8,
    prog_if: u8,
    subclass: u8,
    class_code: u8,
    cache_line_size: u8,
    latency_timer: u8,
    header_type: u8,
    bist: u8,
}

impl CfgCommonHeader {
    const HEADER_TYPE_MASK: u8 = 0b1;
    /* Source: https://wiki.osdev.org/PCI#Configuration_Space */
    const HEADER_TYPE_SINGLE_FUNC_MASK: u8 = 0b1 << 7;
    const VENDOR_ID_NOT_PRESENT: u16 = 0xffff;

    pub unsafe fn is_device_present_at(cfg: *const Self) -> bool {
        unsafe { read_u16(cfg.cast(), 0x00) != Self::VENDOR_ID_NOT_PRESENT }
    }

    pub unsafe fn is_bridge_at(cfg: *const Self) -> bool {
        unsafe { read_u8(cfg.cast(), 0x0e) & Self::HEADER_TYPE_MASK == 0b1 }
    }

    pub unsafe fn is_multi_function_at(cfg: *const Self) -> bool {
        unsafe { read_u8(cfg.cast(), 0x0e) & Self::HEADER_TYPE_SINGLE_FUNC_MASK != 0 }
    }

    pub unsafe fn identifier_at(cfg: *const Self) -> PciIdentifier {
        PciIdentifier {
            vendor_id: unsafe { read_u16(cfg.cast(), 0x00) },
            device_id: unsafe { read_u16(cfg.cast(), 0x02) },
            class_code: unsafe { read_u8(cfg.cast(), 0x0b) },
            subclass: unsafe { read_u8(cfg.cast(), 0x0a) },
            prog_if: unsafe { read_u8(cfg.cast(), 0x09) },
        }
    }

    /// Determines if the PCI(-X/e) device supports capabilities.
    pub unsafe fn capabilities_supported_at(cfg: *const Self) -> bool {
        const CAPABILITIES_SUPPORT_STATUS_BIT: u16 = 1 << 4;

        unsafe { read_u16(cfg.cast(), 0x06) & CAPABILITIES_SUPPORT_STATUS_BIT != 0 }
    }

    /// The capabilities pointer, if the status register advertises a list.
    pub unsafe fn capabilities_offset_at(cfg: *const Self) -> Option<PciCapabilityOffset> {
        if unsafe { Self::capabilities_supported_at(cfg) } {
            Some(unsafe { read_u8(cfg.cast(), 0x34) })
        } else {
            None
        }
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
    capabilities_offset: u8,
    unused0: [u8; 7],
    interrupt_line: u8,
    interrupt_pin: u8,
    bridge_control: u16,
}

#[allow(dead_code)]
impl CfgBridgeHeader {
    pub unsafe fn secondary_bus_num_at(cfg: *const Self) -> u8 {
        unsafe { read_u8(cfg.cast(), 0x19) }
    }

    pub fn is_secondary_bus_pcie(&self) -> bool {
        todo!(
            "Implement this by checking for PCIe capability in the capability list of the \
             bridge's configuration space."
        )
    }

    pub unsafe fn capabilities_offset_at(cfg: *const Self) -> Option<PciCapabilityOffset> {
        unsafe { CfgCommonHeader::capabilities_offset_at(cfg.cast()) }
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
    capabilities_offset: u8,
    _unused0: [u8; 7],
    interrupt_line: u8,
    interrupt_pin: u8,
    _min_grant: u8,
    _max_latency: u8,
}

impl CfgEndpointHeader {
    pub unsafe fn capabilities_offset_at(cfg: *const Self) -> Option<PciCapabilityOffset> {
        unsafe { CfgCommonHeader::capabilities_offset_at(cfg.cast()) }
    }

    /// Read one 32-bit BAR. Out-of-range indices return 0 instead of
    /// panicking, since the index may be derived from firmware-supplied
    /// descriptors. The BARs live at 0x10 and are read one byte at a time so
    /// the access is volatile and alignment-safe.
    pub unsafe fn bar_at(cfg: *const Self, index: usize) -> u32 {
        const BAR_COUNT: usize = 6;
        if index >= BAR_COUNT {
            return 0;
        }
        unsafe { read_u32(cfg.cast(), 0x10 + index * 4) }
    }

    pub unsafe fn interrupt_line_at(cfg: *const Self) -> u8 {
        unsafe { read_u8(cfg.cast(), 0x3c) }
    }
}

/// The configuration space header for a PCIe device, which can be either a bridge or an endpoint
pub union CfgHeader {
    pub common: ManuallyDrop<CfgCommonHeader>, /* For determining header type before safely
                                                * accessing bridge/endpoint-specific fields */
    pub bridge: ManuallyDrop<CfgBridgeHeader>,
    pub endpoint: ManuallyDrop<CfgEndpointHeader>,
}
