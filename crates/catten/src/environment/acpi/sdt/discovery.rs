//! Non-allocating ACPI table discovery.
//!
//! The kernel heap is not available during the earliest boot (the PL011 UART
//! console is initialized before the kernel allocator), so these lookups walk
//! the XSDT directly without allocating. They derive device base addresses that
//! were previously hardcoded to the QEMU `virt` geometry, so the kernel can boot
//! on other ACPI machines (e.g. QEMU `sbsa-ref`, or real ARM servers) where the
//! GIC and UART live at different physical addresses.

#![allow(dead_code)]

use core::mem::size_of;

use crate::{
    environment::acpi::{
        SdtHeader,
        get_xsdp,
    },
    memory::{
        PAddr,
        physical::PhysicalAddress,
    },
};

/// Walk the XSDT and return the physical address of the first table whose
/// four-character signature matches `signature`.
pub fn find_table_physical(signature: [u8; 4]) -> Option<u64> {
    let xsdp = get_xsdp()?;
    let xsdt_addr = unsafe { (*xsdp.as_ptr()).xsdt_address };
    if xsdt_addr == 0 {
        crate::logln!("[acpi-discovery] XSDP has no XSDT address");
        return None;
    }
    let xsdt: &SdtHeader = unsafe { &*PAddr::from(xsdt_addr).into_hhdm_ptr::<SdtHeader>() };
    if !xsdt.validate() {
        crate::logln!("[acpi-discovery] XSDT checksum invalid");
        return None;
    }
    let data_len = xsdt.length as usize - size_of::<SdtHeader>();
    let entry_count = data_len / size_of::<u64>();
    let entries =
        unsafe { (PAddr::from(xsdt_addr) + size_of::<SdtHeader>()).into_hhdm_ptr::<u64>() };
    for i in 0..entry_count {
        let table_addr = unsafe { entries.add(i).read_unaligned() };
        let header: &SdtHeader = unsafe { &*PAddr::from(table_addr).into_hhdm_ptr::<SdtHeader>() };
        if header.signature == signature {
            return Some(table_addr);
        }
    }
    None
}

/// The UART base address published by the SPCR (Serial Port Console
/// Redirection) table, if ACPI is present.
///
/// Layout: `SdtHeader` (36) + `InterfaceType` (1) + reserved (3) + a
/// [`GenericAddressStructure`](super::GenericAddressStructure) whose 64-bit
/// `address` field lives at byte 4 of the GAS.
pub fn spcr_uart_base() -> Option<u64> {
    const SPCR_GAS_OFFSET: u64 = 36 + 1 + 3; // SdtHeader + InterfaceType + reserved
    const GAS_ADDRESS_OFFSET: u64 = 4;
    let spcr = find_table_physical(*b"SPCR")?;
    let addr = spcr + SPCR_GAS_OFFSET + GAS_ADDRESS_OFFSET;
    let base = unsafe { (PAddr::from(addr).into_hhdm_ptr::<u64>()).read_unaligned() };
    (base != 0).then_some(base)
}

/// The console UART's interrupt (a GIC SPI INTID) from the SPCR table, if
/// present. Layout: after the GAS, `InterruptType` (1) + `PCATCompatible` (1),
/// then the 32-bit `Interrupt` field.
pub fn spcr_uart_irq() -> Option<u32> {
    const SPCR_INTERRUPT_OFFSET: u64 = 36 + 1 + 3 + 12 + 1 + 1;
    let spcr = find_table_physical(*b"SPCR")?;
    let irq = unsafe {
        (PAddr::from(spcr + SPCR_INTERRUPT_OFFSET).into_hhdm_ptr::<u32>()).read_unaligned()
    };
    (irq != 0).then_some(irq)
}

/// The GIC distributor and redistributor base addresses from the MADT.
///
/// Returns `(gicd_base, gicr_base)`. The GICD base comes from the GIC
/// Distributor entry; the GICR base from that entry's GICR field, falling back
/// to a GIC Redistributor entry if the field is absent.
pub fn madt_gic_bases() -> Option<(u64, u64)> {
    const GIC_CPU_INTERFACE: u8 = 0x0b;
    const GIC_DISTRIBUTOR: u8 = 0xc;
    const GIC_REDISTRIBUTOR: u8 = 0xe;
    const MADT_HEADER_SIZE: u64 = 36 + 4 + 4; // SdtHeader + LAPIC address + flags
    const GICD_ENTRY_BASE_OFFSET: u64 = 8;
    // A GICC structure carries the redistributor base for that CPU at offset
    // 60. The system-wide GICR structure carries the discovery-range base at
    // offset 4.  A GIC Distributor structure is only 24 bytes long and has no
    // redistributor field; reading one at offset 44 crosses into the following
    // MADT entry and can silently turn unrelated bytes into a bogus MMIO base.
    const GICC_ENTRY_GICR_OFFSET: u64 = 60;
    const GICR_ENTRY_BASE_OFFSET: u64 = 4;

    let madt = find_table_physical(*b"APIC")?;
    let header: &SdtHeader = unsafe { &*PAddr::from(madt).into_hhdm_ptr::<SdtHeader>() };
    let end = madt + header.length as u64;
    let mut ptr = madt + MADT_HEADER_SIZE;
    let mut gicd = 0u64;
    let mut gicr = 0u64;
    while ptr < end {
        let entry = unsafe { PAddr::from(ptr).into_hhdm_ptr::<u8>() };
        let entry_type = unsafe { *entry };
        let entry_len = unsafe { *entry.add(1) } as u64;
        if entry_len == 0 || ptr + entry_len > end {
            break;
        }
        match entry_type {
            GIC_CPU_INTERFACE if entry_len >= GICC_ENTRY_GICR_OFFSET + 8 => {
                let rbase = unsafe {
                    (PAddr::from(ptr + GICC_ENTRY_GICR_OFFSET).into_hhdm_ptr::<u64>())
                        .read_unaligned()
                };
                // Per-CPU GICC entries normally enumerate consecutive
                // redistributors.  Retain the lowest base as the start of the
                // discovery range used by the GIC driver.
                if rbase != 0 && (gicr == 0 || rbase < gicr) {
                    gicr = rbase;
                }
            }
            GIC_DISTRIBUTOR => {
                gicd = unsafe {
                    (PAddr::from(ptr + GICD_ENTRY_BASE_OFFSET).into_hhdm_ptr::<u64>())
                        .read_unaligned()
                };
            }
            GIC_REDISTRIBUTOR => {
                let rbase = unsafe {
                    (PAddr::from(ptr + GICR_ENTRY_BASE_OFFSET).into_hhdm_ptr::<u64>())
                        .read_unaligned()
                };
                if rbase != 0 {
                    gicr = rbase;
                }
            }
            _ => {}
        }
        ptr += entry_len;
    }
    (gicd != 0 && gicr != 0).then_some((gicd, gicr))
}

/// The GIC ITS base address from the MADT, if the platform publishes one.
///
/// QEMU `sbsa-ref` emits a GIC ITS entry whose 64-bit base sits at offset 8.
/// MADT type 0x0E is the distinct GIC Redistributor structure and must never be
/// interpreted as an ITS: doing so directs ITS command writes into GICR MMIO.
pub fn madt_its_base() -> Option<u64> {
    const GIC_ITS: u8 = 0x0f;
    const MADT_HEADER_SIZE: u64 = 36 + 4 + 4;
    const ENTRY_BASE_OFFSET: u64 = 8;

    let madt = find_table_physical(*b"APIC")?;
    let header: &SdtHeader = unsafe { &*PAddr::from(madt).into_hhdm_ptr::<SdtHeader>() };
    let end = madt + header.length as u64;
    let mut ptr = madt + MADT_HEADER_SIZE;
    while ptr < end {
        let entry = unsafe { PAddr::from(ptr).into_hhdm_ptr::<u8>() };
        let entry_type = unsafe { *entry };
        let entry_len = unsafe { *entry.add(1) } as u64;
        if entry_len == 0 || ptr + entry_len > end {
            break;
        }
        if entry_type == GIC_ITS {
            let base = unsafe {
                (PAddr::from(ptr + ENTRY_BASE_OFFSET).into_hhdm_ptr::<u64>()).read_unaligned()
            };
            // A plausible ITS MMIO base lives in the server MMIO window (not
            // the garbage from a misaligned read or a RAM address).
            if base != 0 && base < 0x100_0000_0000 {
                return Some(base);
            }
        }
        ptr += entry_len;
    }
    None
}

/// The first I/O APIC base address and its global-system-interrupt (GSI) base
/// from the MADT. Returns `(ioapic_address, gsi_base)`.
///
/// MADT type 0x01 (I/O APIC) is a 12-byte structure: the 32-bit MMIO base at
/// offset 4 and the 32-bit GSI base at offset 8.
pub fn madt_ioapic() -> Option<(u64, u32)> {
    const IO_APIC: u8 = 0x01;
    const MADT_HEADER_SIZE: u64 = 36 + 4 + 4; // SdtHeader + LAPIC address + flags
    const ENTRY_ADDRESS_OFFSET: u64 = 4;
    const ENTRY_GSI_BASE_OFFSET: u64 = 8;

    let madt = find_table_physical(*b"APIC")?;
    let header: &SdtHeader = unsafe { &*PAddr::from(madt).into_hhdm_ptr::<SdtHeader>() };
    let end = madt + header.length as u64;
    let mut ptr = madt + MADT_HEADER_SIZE;
    while ptr < end {
        let entry = unsafe { PAddr::from(ptr).into_hhdm_ptr::<u8>() };
        let entry_type = unsafe { *entry };
        let entry_len = unsafe { *entry.add(1) } as u64;
        if entry_len == 0 || ptr + entry_len > end {
            break;
        }
        if entry_type == IO_APIC {
            let address = unsafe {
                (PAddr::from(ptr + ENTRY_ADDRESS_OFFSET).into_hhdm_ptr::<u32>()).read_unaligned()
            };
            let gsi_base = unsafe {
                (PAddr::from(ptr + ENTRY_GSI_BASE_OFFSET).into_hhdm_ptr::<u32>()).read_unaligned()
            };
            if address != 0 {
                return Some((address as u64, gsi_base));
            }
        }
        ptr += entry_len;
    }
    None
}
