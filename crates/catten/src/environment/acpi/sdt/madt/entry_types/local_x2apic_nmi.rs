use crate::environment::acpi::sdt::madt::entry_types::MadtEntryType;
use crate::environment::acpi::sdt::madt::interrupt_flags::NmiSrcFlags;

/* The entries specified by this structure need to raise an NMI on the host processor (vector 2) */
pub struct LocalX2ApicNmiEntry {
    entry_type: MadtEntryType,
    entry_length: u8,
    flags: NmiSrcFlags,
    acpi_proc_uid: u32,
    local_interrupt_entry: u8,
}
