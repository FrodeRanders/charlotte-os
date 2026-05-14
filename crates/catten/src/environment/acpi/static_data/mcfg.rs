use alloc::vec::Vec;

use crate::drivers::busses::pcie::PcieSegment;
use crate::environment::acpi::{AcpiTableType, TABLE_MAP};
use crate::memory::PAddr;

const MCFG_HEADER_SIZE: usize = 43;
const MCFG_LEN_OFFSET: usize = 4;
const MCFG_ENTRY_SIZE: usize = 16;

pub fn parse_mcfg() -> Vec<PcieSegment> {
    TABLE_MAP
        .get(&AcpiTableType::MCFG)
        .map(|table| {
            let table_ptr = table.as_ptr();
            let entry_count = (unsafe { *(table_ptr.add(MCFG_LEN_OFFSET) as *const u32) } as usize
                - MCFG_HEADER_SIZE)
                / MCFG_ENTRY_SIZE;
            let mut segments = Vec::with_capacity(entry_count);
            for i in 0..entry_count {
                let entry_ptr = unsafe { table_ptr.add(MCFG_HEADER_SIZE + i * MCFG_ENTRY_SIZE) };
                segments.push(parse_mcfg_entry(entry_ptr as *const McfgEntry));
            }
            segments
        })
        .unwrap_or_default()
}

struct McfgEntry {
    ecam_base: PAddr,
    pcie_segment_num: u16,
    start_bus_num: u8,
    end_bus_num: u8,
    _reserved: u32,
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
