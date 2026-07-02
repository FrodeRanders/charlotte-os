use crate::environment::acpi::sdt::madt::GlobalSystemInterrupt;
use crate::environment::acpi::sdt::madt::entry_types::MadtEntryType;
use crate::environment::acpi::sdt::madt::interrupt_flags::NmiSrcFlags;

#[derive(Debug, PartialEq, Eq)]
#[repr(C, packed)]
pub struct NmiSourceEntry {
    entry_type: MadtEntryType,
    length: u8,
    flags: NmiSrcFlags,
    global_system_interrupt: GlobalSystemInterrupt,
}
