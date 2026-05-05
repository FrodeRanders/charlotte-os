use alloc::vec::Vec;

use crate::drivers::busses::pcie::PcieSegment;
use crate::memory::PAddr;

const MCFG_HEADER_SIZE: usize = 43;
const MCFG_LEN_OFFSET: usize = 4;
const MCFG_ENTRY_SIZE: usize = 16;

pub fn parse_mcfg() -> Vec<PcieSegment> {
    todo!("Get the MCFG table from the ACPI table map and parse it.");
}

struct McfgEntry {
    ecam_base: PAddr,
    pcie_segment_num: u16,
    start_bus_num: u8,
    end_bus_num: u8,
    reserved: u32,
}

fn parse_mcfg_entry(entry: *const McfgEntry) -> PcieSegment {
    unsafe {
        PcieSegment::new(
            (*entry).pcie_segment_num,
            (*entry).ecam_base,
            (*entry).start_bus_num,
            (*entry).end_bus_num,
        )
    }
}
