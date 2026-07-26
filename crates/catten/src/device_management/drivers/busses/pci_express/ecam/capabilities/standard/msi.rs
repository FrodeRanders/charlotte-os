#![cfg_attr(target_arch = "x86_64", allow(dead_code))]

/// The address/data pair written by a PCI function when it raises an MSI.
#[derive(Debug, Clone, Copy)]
pub struct MsiMessage {
    pub address: u64,
    pub data: u32,
    pub intid: u32,
}
