//! # PCI Express Enhanced Configuration Access Mechanism (ECAM)

#[repr(C, packed)]
pub struct PcieCfgCommonHeader {
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

#[repr(C, packed)]
pub struct PcieCfgBridgeHeader {
    common: PcieCfgCommonHeader,
    bar: [u32; 2],
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
pub struct PcieCfgEndpointHeader {
    common: PcieCfgCommonHeader,
    bar: [u32; 6],
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
