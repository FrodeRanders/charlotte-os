pub mod msi;
pub mod msix;
/// PCI local bus capability IDs which are also used with PCI Express.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
pub enum PciCapabilityId {
    Null = 0x00,
    PowerManagement = 0x01,
    MessageSignaledInterrupts = 0x05,
    /// Subsystem Vendor ID and Subsystem Device ID
    SubsystemVidDid = 0x0d,
    PciExpress = 0x10,
    // MSI-X
    MessageSignaledInterruptsExtended = 0x11,
}

pub type PciCapabilityOffset = u8;
