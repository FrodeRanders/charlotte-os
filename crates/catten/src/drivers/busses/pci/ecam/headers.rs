use core::fmt::Debug;
use core::mem::ManuallyDrop;

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

    pub fn is_device_present(&self) -> bool {
        self.vendor_id != Self::VENDOR_ID_NOT_PRESENT
    }

    pub fn is_bridge(&self) -> bool {
        self.header_type & Self::HEADER_TYPE_MASK == 0b1
    }

    pub fn is_multi_function(&self) -> bool {
        self.header_type & Self::HEADER_TYPE_SINGLE_FUNC_MASK != 0
    }

    pub fn get_vendor_id(&self) -> u16 {
        self.vendor_id
    }

    pub fn get_device_id(&self) -> u16 {
        self.device_id
    }

    pub fn get_class(&self) -> Class {
        Class::new(self.class_code, self.subclass, self.prog_if)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Class {
    class_code: u8,
    subclass: u8,
    prog_if: u8,
}

impl Class {
    pub fn new(class_code: u8, subclass: u8, prog_if: u8) -> Self {
        Class {
            class_code,
            subclass,
            prog_if,
        }
    }
}

impl Debug for Class {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "PCI Express Device Class\n    class_code: {:#x}\n    subclass: {:#x}\n    prog_if: \
             {:#x}",
            self.class_code, self.subclass, self.prog_if
        )
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
    unused0: [u8; 7],
    interrupt_line: u8,
    interrupt_pin: u8,
    bridge_control: u16,
}

impl CfgBridgeHeader {
    pub fn get_secondary_bus_num(&self) -> u8 {
        self.secondary_bus_num
    }

    pub fn is_secondary_bus_pcie(&self) -> bool {
        todo!(
            "Implement this by checking for PCIe capability in the capability list of the \
             bridge's configuration space."
        )
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
