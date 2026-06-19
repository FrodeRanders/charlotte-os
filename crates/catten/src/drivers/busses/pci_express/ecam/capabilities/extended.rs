use super::*;

#[repr(u16)]
pub enum ExtendedCapabilityId {
    Null = 0x0000,
    AdvancedErrorReporting = 0x0001,
    AccessControlServices = 0x000d,
    // SR-IOV
    SingleRootIoVirtualization = 0x0010,
    ResizeableBaseAddressRegisters = 0x0011,
}
