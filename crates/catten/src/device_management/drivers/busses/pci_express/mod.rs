pub mod device_class;
pub(crate) mod ecam;
pub mod topology;

#[derive(Debug)]
pub enum Error {
    InvalidLocation,
    PciCapabilitiesNotSupported,
    NotPciExpress,
    PciCapabilityNotFound,
    ValueOutOfRange,
}

#[allow(unused)]
const MAX_SEGMENT_GROUPS: usize = 1 << 16; // 65536 segment groups
const MAX_DEVICES_PER_BUS: usize = 32;
const MAX_FUNCTIONS_PER_DEVICE: usize = 8;
/// Maximum PCIe bridge nesting depth enumerated below a root bus. Firmware
/// can describe cycles or absurdly deep hierarchies; enumeration stops
/// descending once this bound is reached.
pub(crate) const MAX_PCIE_BRIDGE_DEPTH: u8 = 8;
