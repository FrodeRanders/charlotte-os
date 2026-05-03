use alloc::vec::Vec;

use uacpi_raw::*;

use crate::cpu::isa::interface::memory::address::VirtualAddress;
use crate::drivers::busses::pcie::PcieSegment;
use crate::environment::acpi;
use crate::memory::{PAddr, VAddr};

pub fn get_pcie_segments() -> Vec<PcieSegment> {
    let mcfg_addrs =
        acpi::find_table_type(acpi::AcpiTableType::MCFG).expect("Failed to find MCFG table");
    if mcfg_addrs.len() != 1 {
        panic!("[ACPI] {} MCFG tables found, expected exactly one.", mcfg_addrs.len());
    }
    parse_mcfg(mcfg_addrs[0].into())
}

const MCFG_HEADER_SIZE: usize = 43;
const MCFG_LEN_OFFSET: usize = 4;
const MCFG_ENTRY_SIZE: usize = 16;

fn parse_mcfg(mcfg_info_ptr: *const uacpi_table) -> Vec<PcieSegment> {
    let mut segments = Vec::new();
    unsafe {
        let mcfg_vaddr = VAddr::from((*mcfg_info_ptr).__bindgen_anon_1.virt_addr);
        let mcfg_len = *(mcfg_vaddr.into_mut::<u32>().byte_add(MCFG_LEN_OFFSET)) as usize;
        let entry_count = (mcfg_len - MCFG_HEADER_SIZE) / MCFG_ENTRY_SIZE;
        let mcfg_entries_ptr = mcfg_vaddr.into_mut::<McfgEntry>().byte_add(MCFG_HEADER_SIZE);
        for i in 0..entry_count {
            segments.push(parse_mcfg_entry(mcfg_entries_ptr.add(i)))
        }
    }
    segments
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
