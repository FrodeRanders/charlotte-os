use crate::environment::acpi::sdt::madt::GlobalSystemInterrupt;
use crate::environment::acpi::sdt::madt::entry_types::MadtEntryType;
use crate::memory::PAddr;

type IoApicId = u8;

#[derive(Debug)]
#[repr(C, packed)]
pub struct IoApicEntry {
    entry_type: MadtEntryType,
    length: u8,
    ioapic_id: IoApicId,
    reserved: u8,
    ioapic_address: u32,
    global_system_interrupt_base: GlobalSystemInterrupt,
}

impl IoApicEntry {
    pub fn id(&self) -> IoApicId {
        self.ioapic_id
    }

    pub fn address(&self) -> PAddr {
        PAddr::from(self.ioapic_address as u64)
    }

    pub fn gsi_base(&self) -> GlobalSystemInterrupt {
        self.global_system_interrupt_base
    }
}
