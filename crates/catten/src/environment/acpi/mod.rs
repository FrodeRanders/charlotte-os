use alloc::alloc::{alloc_zeroed, dealloc};
use alloc::vec::Vec;
use core::alloc::Layout;

use uacpi_raw::*;

use crate::memory::{PAddr, VAddr};

pub mod uacpi_kernel;

pub fn get_pcie_ecam_bases() -> Vec<(u16, PAddr)> {
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

fn parse_mcfg(mcfg_ptr: *const uacpi_table) -> Vec<(u16, PAddr)> {
    let mut ecams = Vec::new();
    unsafe {
        let mcfg_vaddr = VAddr::from((*mcfg_ptr).__bindgen_anon_1.virt_addr);
        todo!(
            "Parse the MCFG table to get the base physical address for each ECAM and write them \
             to the vector keyed by their segment group number."
        );
    }
    ecams
}
