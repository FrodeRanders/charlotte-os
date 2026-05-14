use alloc::vec::Vec;
use core::mem::offset_of;
use core::ptr::read_unaligned;

use crate::drivers::busses::pcie::PcieSegment;
use crate::environment::acpi::{AcpiTableType, SdtHeader, TABLE_MAP};
use crate::memory::PAddr;
use crate::memory::physical::PhysicalAddress;
use crate::println;

const MCFG_HEADER_SIZE: usize = core::mem::size_of::<SdtHeader>();
const MCFG_ENTRY_SIZE: usize = core::mem::size_of::<McfgEntry>();

pub fn parse_mcfg() -> Vec<PcieSegment> {
    println!("Parsing MCFG table...");
    TABLE_MAP
        .get(&AcpiTableType::MCFG)
        .map(|table| {
            println!("MCFG tables found at the following addresses: {:?}", (table));
            let table_ptr = unsafe { table[0].into_hhdm_ptr::<SdtHeader>() };
            let entry_count = (unsafe {
                read_unaligned(table_ptr.add(offset_of!(SdtHeader, length)) as *const u32)
            } as usize
                - MCFG_HEADER_SIZE)
                / MCFG_ENTRY_SIZE;
            println!("MCFG entry count: {}", (entry_count));
            let mut segments = Vec::with_capacity(entry_count);
            for i in 0..entry_count {
                let entry_ptr = unsafe { table_ptr.add(MCFG_HEADER_SIZE + i * MCFG_ENTRY_SIZE) }
                    as *const McfgEntry;
                segments.push(parse_mcfg_entry(entry_ptr));
            }
            segments
        })
        .unwrap_or_default()
}

#[derive(Debug)]
#[repr(C, packed)]
struct McfgEntry {
    ecam_base: PAddr,
    pcie_segment_num: u16,
    start_bus_num: u8,
    end_bus_num: u8,
    _reserved: u32,
}

fn parse_mcfg_entry(entry: *const McfgEntry) -> PcieSegment {
    println!("Parsing MCFG entry at address {:p}", entry);
    unsafe {
        PcieSegment::new(
            read_unaligned(entry).pcie_segment_num,
            read_unaligned(entry).ecam_base,
            read_unaligned(entry).start_bus_num,
            read_unaligned(entry).end_bus_num,
        )
    }
}
