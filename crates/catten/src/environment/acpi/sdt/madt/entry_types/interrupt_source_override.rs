pub struct InterruptSourceOverrideEntry {
    entry_type: MadtEntryType,
    length: u8,
    bus_source: u8,
    irq_source: u8,
    global_system_interrupt: GlobalSystemInterrupt,
    flags: NmiSrcFlags,
}
