//! Minimal, bounds-checked ACPI DMAR discovery for Intel VT-d.
//!
//! CharlotteOS currently drives a single DMA remapping unit. The parser
//! returns the first DRHD (DMA Remapping Hardware unit Definition) and its
//! INCLUDE_PCI_ALL flag; the VT-d source id is the PCI requester id itself
//! (bus:device:function), so no separate id translation table is required.

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

const DMAR_HEADER_SIZE: usize = size_of::<SdtHeader>() + 12;
const DRHD_TYPE: u16 = 0;
const DRHD_HEADER_SIZE: usize = 16;
const DRHD_FLAG_INCLUDE_PCI_ALL: u8 = 1;

#[derive(Debug, Clone, Copy)]
pub struct DmarConfig {
    pub base: usize,
    pub segment: u16,
    pub include_pci_all: bool,
}

fn read_u16(base: *const u8, offset: usize) -> u16 {
    unsafe { ptr::read_unaligned(base.add(offset).cast()) }
}

fn read_u32(base: *const u8, offset: usize) -> u32 {
    unsafe { ptr::read_unaligned(base.add(offset).cast()) }
}

fn read_u64(base: *const u8, offset: usize) -> u64 {
    unsafe { ptr::read_unaligned(base.add(offset).cast()) }
}

fn read_u8(base: *const u8, offset: usize) -> u8 {
    unsafe { ptr::read_unaligned(base.add(offset)) }
}

/// Discover the first DMA remapping unit (DRHD).
pub fn discover_vtd() -> Option<DmarConfig> {
    let table = *acpi::find_table_type(AcpiTableType::DMAR).ok()?.first()?;
    let base = unsafe { table.into_hhdm_ptr::<u8>() };
    let header = unsafe { &*base.cast::<SdtHeader>() };
    if !header.validate() {
        return None;
    }
    let table_len = header.length as usize;
    if table_len < DMAR_HEADER_SIZE {
        return None;
    }

    let mut offset = DMAR_HEADER_SIZE;
    while offset + 4 <= table_len {
        let kind = read_u16(base, offset);
        let length = read_u16(base, offset + 2) as usize;
        if length < 4 || offset.checked_add(length)? > table_len {
            return None;
        }
        if kind == DRHD_TYPE {
            if length < DRHD_HEADER_SIZE {
                return None;
            }
            let flags = read_u8(base, offset + 4);
            let segment = read_u16(base, offset + 6);
            let register_base = usize::try_from(read_u64(base, offset + 8)).ok()?;
            return Some(DmarConfig {
                base: register_base,
                segment,
                include_pci_all: flags & DRHD_FLAG_INCLUDE_PCI_ALL != 0,
            });
        }
        offset = offset.checked_add(length)?;
    }
    None
}
