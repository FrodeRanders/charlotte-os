//! PCI MSI-X capability and table programming.
#![cfg_attr(target_arch = "x86_64", allow(dead_code))]

use core::sync::atomic::{
    Ordering,
    fence,
};

use crate::{
    cpu::isa::interface::memory::{
        AddressSpaceInterface,
        address::PhysicalAddress,
    },
    device_management::drivers::busses::pci_express::ecam::{
        capabilities::standard::msi::MsiMessage,
        pcie::PcieCfgSpace,
    },
    memory::{
        AddressSpace,
        PAddr,
    },
};

const CAP_ID_MSIX: u8 = 0x11;
const MSIX_CONTROL_ENABLE: u16 = 1 << 15;
const MSIX_CONTROL_FUNCTION_MASK: u16 = 1 << 14;
const TABLE_BIR_MASK: u32 = 0x7;
const TABLE_OFFSET_MASK: u32 = !0x7;
const VECTOR_MASKED: u32 = 1;
const PAGE_SIZE: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    CapabilityMissing,
    MalformedCapabilityList,
    InvalidBar,
    MmioMapFailed,
}

unsafe fn read_u8(base: *const u8, offset: usize) -> u8 {
    unsafe { core::ptr::read_volatile(base.add(offset)) }
}

unsafe fn read_u16(base: *const u8, offset: usize) -> u16 {
    unsafe { core::ptr::read_volatile(base.add(offset).cast::<u16>()) }
}

unsafe fn write_u16(base: *mut u8, offset: usize, value: u16) {
    unsafe { core::ptr::write_volatile(base.add(offset).cast::<u16>(), value) }
}

unsafe fn read_u32(base: *const u8, offset: usize) -> u32 {
    unsafe { core::ptr::read_volatile(base.add(offset).cast::<u32>()) }
}

fn find_msix(cfg: *const u8) -> Result<usize, Error> {
    let status = unsafe { read_u16(cfg, 0x06) };
    if status & (1 << 4) == 0 {
        return Err(Error::CapabilityMissing);
    }
    let mut offset = unsafe { read_u8(cfg, 0x34) } as usize;
    let mut visited = [false; 256];
    while offset != 0 {
        if offset < 0x40 || offset > 0xfc || offset & 0x3 != 0 || visited[offset] {
            return Err(Error::MalformedCapabilityList);
        }
        visited[offset] = true;
        if unsafe { read_u8(cfg, offset) } == CAP_ID_MSIX {
            return Ok(offset);
        }
        offset = unsafe { read_u8(cfg, offset + 1) } as usize;
    }
    Err(Error::CapabilityMissing)
}

fn decode_bar(cfg: *const u8, bir: usize) -> Result<u64, Error> {
    if bir >= 6 {
        return Err(Error::InvalidBar);
    }
    let low = unsafe { read_u32(cfg, 0x10 + bir * 4) };
    if low & 1 != 0 {
        return Err(Error::InvalidBar);
    }
    let memory_type = (low >> 1) & 0x3;
    let mut address = (low & 0xffff_fff0) as u64;
    if memory_type == 0x2 {
        if bir == 5 {
            return Err(Error::InvalidBar);
        }
        let high = unsafe { read_u32(cfg, 0x10 + (bir + 1) * 4) };
        address |= (high as u64) << 32;
    }
    if address == 0 {
        return Err(Error::InvalidBar);
    }
    Ok(address)
}

/// Program MSI-X table entry zero and enable MSI-X for a PCI function.
///
/// Configuration space must already be mapped through ECAM. The table BAR is
/// mapped only in the current kernel address space and is never delegated to
/// the userspace driver.
pub fn program_vector0(cfg_space: *mut PcieCfgSpace, message: MsiMessage) -> Result<(), Error> {
    let cfg = cfg_space.cast::<u8>();
    let cap = find_msix(cfg)?;
    let table = unsafe { read_u32(cfg, cap + 4) };
    let bir = (table & TABLE_BIR_MASK) as usize;
    let table_offset = (table & TABLE_OFFSET_MASK) as u64;
    let table_phys = decode_bar(cfg, bir)?.checked_add(table_offset).ok_or(Error::InvalidBar)?;
    let page_base = table_phys & !(PAGE_SIZE - 1);
    let page_offset = (table_phys - page_base) as usize;
    if page_offset + 16 > PAGE_SIZE as usize {
        return Err(Error::InvalidBar);
    }

    let mut current = AddressSpace::get_current();
    current
        .map_mmio_region(page_base as usize, PAGE_SIZE as usize)
        .map_err(|_| Error::MmioMapFailed)?;
    let entry = unsafe { PAddr::from(page_base).into_hhdm_mut::<u8>().add(page_offset) };

    let mut control = unsafe { read_u16(cfg, cap + 2) };
    control |= MSIX_CONTROL_FUNCTION_MASK;
    control &= !MSIX_CONTROL_ENABLE;
    unsafe { write_u16(cfg, cap + 2, control) };

    unsafe {
        core::ptr::write_volatile(entry.add(12).cast::<u32>(), VECTOR_MASKED);
        core::ptr::write_volatile(entry.cast::<u32>(), message.address as u32);
        core::ptr::write_volatile(entry.add(4).cast::<u32>(), (message.address >> 32) as u32);
        core::ptr::write_volatile(entry.add(8).cast::<u32>(), message.data);
    }
    fence(Ordering::SeqCst);

    // Enable memory-space decoding and bus mastering.
    let command = unsafe { read_u16(cfg, 0x04) };
    unsafe { write_u16(cfg, 0x04, command | (1 << 1) | (1 << 2)) };

    control |= MSIX_CONTROL_ENABLE;
    control &= !MSIX_CONTROL_FUNCTION_MASK;
    unsafe { write_u16(cfg, cap + 2, control) };
    fence(Ordering::SeqCst);
    unsafe { core::ptr::write_volatile(entry.add(12).cast::<u32>(), 0) };
    fence(Ordering::SeqCst);
    Ok(())
}
