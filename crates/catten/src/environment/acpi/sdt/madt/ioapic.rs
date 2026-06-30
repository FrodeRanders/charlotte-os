use super::GlobalSystemInterruptVector;
use crate::memory::PAddr;

type IoApicId = u8;

#[derive(Debug)]
#[repr(C, packed)]
pub struct IoApic {
    entry_type: u8,
    length: u8,
    ioapic_id: IoApicId,
    reserved: u8,
    ioapic_address: u32,
    global_system_interrupt_base: GlobalSystemInterruptVector,
}

impl IoApic {
    pub fn id(&self) -> IoApicId {
        self.ioapic_id
    }

    pub fn address(&self) -> PAddr {
        PAddr::from(self.ioapic_address as u64)
    }

    pub fn gsi_base(&self) -> GlobalSystemInterruptVector {
        self.global_system_interrupt_base
    }
}
