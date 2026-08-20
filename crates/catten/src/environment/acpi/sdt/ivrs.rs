//! Minimal, bounds-checked ACPI IVRS discovery for AMD-Vi.
//!
//! CharlotteOS currently drives a single IOMMU, so the parser returns the
//! register base of the first IVHD (I/O Virtualization Hardware Definition)
//! block. The AMD-Vi device id is the PCI requester id itself, so no separate
//! id translation table is required.

use core::{
    mem::size_of,
    ptr,
};

use crate::{
    environment::acpi::{
        self,
        AcpiTableType,
        SdtHeader,
    },
    memory::physical::PhysicalAddress,
};

const IVRS_HEADER_SIZE: usize = size_of::<SdtHeader>() + 12;
const IVHD_TYPE: u8 = 0x10;

#[derive(Debug, Clone, Copy)]
pub struct IvrsConfig {
    pub base: usize,
}

fn read_u16(base: *const u8, offset: usize) -> u16 {
    unsafe { ptr::read_unaligned(base.add(offset).cast()) }
}

fn read_u64(base: *const u8, offset: usize) -> u64 {
    unsafe { ptr::read_unaligned(base.add(offset).cast()) }
}

fn read_u8(base: *const u8, offset: usize) -> u8 {
    unsafe { ptr::read_unaligned(base.add(offset)) }
}

/// Discover the first I/O Virtualization Hardware Definition block.
pub fn discover_amd_vi() -> Option<IvrsConfig> {
    let table = *acpi::find_table_type(AcpiTableType::IVRS).ok()?.first()?;
    let base = unsafe { table.into_hhdm_ptr::<u8>() };
    let header = unsafe { &*base.cast::<SdtHeader>() };
    if !header.validate() {
        return None;
    }
    let table_len = header.length as usize;
    if table_len < IVRS_HEADER_SIZE {
        return None;
    }

    let mut offset = IVRS_HEADER_SIZE;
    while offset + 4 <= table_len {
        let kind = read_u8(base, offset);
        let length = read_u16(base, offset + 2) as usize;
        if length < 4 || offset.checked_add(length)? > table_len {
            return None;
        }
        if kind == IVHD_TYPE {
            if length < 24 {
                return None;
            }
            let register_base = usize::try_from(read_u64(base, offset + 8)).ok()?;
            return Some(IvrsConfig {
                base: register_base,
            });
        }
        offset = offset.checked_add(length)?;
    }
    None
}
