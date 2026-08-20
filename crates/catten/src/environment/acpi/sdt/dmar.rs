//! Minimal, bounds-checked ACPI DMAR discovery for Intel VT-d.
//!
//! CharlotteOS currently drives a single DMA remapping unit. The parser
//! returns the first DRHD (DMA Remapping Hardware unit Definition) and can
//! determine whether that unit covers a directly attached PCI requester from
//! either INCLUDE_PCI_ALL or its endpoint device scopes. The VT-d source id is
//! the PCI requester id itself (bus:device:function), so no separate id
//! translation table is required.

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
const DEVICE_SCOPE_HEADER_SIZE: usize = 6;
const DEVICE_SCOPE_PCI_ENDPOINT: u8 = 1;

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

/// Whether `config`'s DRHD covers a PCI source ID. CharlotteOS currently has
/// no PCI bridge-routing model for DMAR paths, so only a one-hop endpoint path
/// is accepted here. INCLUDE_PCI_ALL remains the general case.
pub fn covers_requester(config: DmarConfig, requester_id: u16) -> bool {
    if config.segment != 0 {
        return false;
    }
    if config.include_pci_all {
        return true;
    }

    let Some(table) =
        acpi::find_table_type(AcpiTableType::DMAR).ok().and_then(|tables| tables.first().copied())
    else {
        return false;
    };
    let base = unsafe { table.into_hhdm_ptr::<u8>() };
    let header = unsafe { &*base.cast::<SdtHeader>() };
    if !header.validate() {
        return false;
    }
    let table_len = header.length as usize;
    let mut offset = DMAR_HEADER_SIZE;
    while offset + 4 <= table_len {
        let kind = read_u16(base, offset);
        let length = read_u16(base, offset + 2) as usize;
        let Some(end) = offset.checked_add(length) else {
            return false;
        };
        if length < 4 || end > table_len {
            return false;
        }
        if kind == DRHD_TYPE && length >= DRHD_HEADER_SIZE {
            let segment = read_u16(base, offset + 6);
            let register_base = read_u64(base, offset + 8) as usize;
            if segment == config.segment && register_base == config.base {
                if read_u8(base, offset + 4) & DRHD_FLAG_INCLUDE_PCI_ALL != 0 {
                    return true;
                }
                let mut scope = offset + DRHD_HEADER_SIZE;
                while scope + DEVICE_SCOPE_HEADER_SIZE <= end {
                    let scope_type = read_u8(base, scope);
                    let scope_len = read_u8(base, scope + 1) as usize;
                    let Some(scope_end) = scope.checked_add(scope_len) else {
                        return false;
                    };
                    if scope_len < DEVICE_SCOPE_HEADER_SIZE || scope_end > end {
                        return false;
                    }
                    let path_len = scope_len - DEVICE_SCOPE_HEADER_SIZE;
                    if scope_type == DEVICE_SCOPE_PCI_ENDPOINT && path_len == 2 {
                        let start_bus = read_u8(base, scope + 5);
                        let device = read_u8(base, scope + 6);
                        let function = read_u8(base, scope + 7);
                        let scoped_id =
                            ((start_bus as u16) << 8) | ((device as u16) << 3) | function as u16;
                        if scoped_id == requester_id {
                            return true;
                        }
                    }
                    scope = scope_end;
                }
            }
        }
        offset = end;
    }
    false
}
