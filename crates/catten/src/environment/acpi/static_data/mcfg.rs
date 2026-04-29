use alloc::alloc::{alloc_zeroed, dealloc};
use alloc::vec::Vec;
use core::alloc::Layout;

use uacpi_raw::*;

use crate::cpu::isa::interface::memory::address::VirtualAddress;
use crate::device_manager::pcie::PcieSegment;
use crate::memory::{PAddr, VAddr};

pub fn get_pcie_segments() -> Vec<PcieSegment> {
    unsafe {
        // We allocate this using the low level interface because we're interfacing with C and
        // this is easier than boxing, unboxing, and reboxing.
        let uacpi_table_layout =
            Layout::from_size_align_unchecked(size_of::<uacpi_table>(), align_of::<uacpi_table>());
        let mcfg_ptr = alloc_zeroed(Layout::from_size_align_unchecked(
            size_of::<uacpi_table>(),
            align_of::<uacpi_table>(),
        )) as *mut uacpi_table;

        uacpi_table_find_by_signature(b"MCFG\0".as_ptr() as *const i8, mcfg_ptr);
        let ret = parse_mcfg(mcfg_ptr);
        dealloc(mcfg_ptr as *mut u8, uacpi_table_layout);
        ret
    }
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
