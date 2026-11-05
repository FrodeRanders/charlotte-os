//! Validate all firmware ECAM regions before mapping or topology publication.
use alloc::vec::Vec;

use crate::{
    cpu::isa::memory::address::PADDR_MASK,
    device_management::drivers::busses::pci_express::topology::{
        PcieSegmentGroup,
        enumeration::Budget,
    },
    environment::acpi::{
        AcpiTableType,
        MAX_ACPI_TABLE_LEN,
        SdtHeader,
        TABLE_MAP,
    },
    memory::{
        PAddr,
        physical::PhysicalAddress,
    },
};
mod regions;
const _: () = assert!(regions::MAX_TABLE_BYTES == MAX_ACPI_TABLE_LEN as usize);

pub fn parse_mcfg() -> Vec<PcieSegmentGroup> {
    let mut regions = regions::Regions::new();
    if let Some(tables) = TABLE_MAP.get(&AcpiTableType::MCFG) {
        for table in tables {
            let pointer = unsafe { table.into_hhdm_ptr::<SdtHeader>() };
            let header = unsafe { pointer.read_unaligned() };
            let len = header.length as usize;
            assert!(
                (size_of::<SdtHeader>()..=MAX_ACPI_TABLE_LEN as usize).contains(&len),
                "invalid MCFG declared length"
            );
            // Trusted boot firmware supplies the mapped table backing. Bound its
            // slice first; the pure parser checks full geometry and checksum.
            let bytes = unsafe { core::slice::from_raw_parts(pointer.cast::<u8>(), len) };
            regions
                .append_table(bytes, *PADDR_MASK as u64)
                .unwrap_or_else(|error| panic!("MCFG preparation rejected: {error:?}"));
        }
    }
    // No ECAM mapping or endpoint construction until every entry is accepted.
    let mut segments = Vec::new();
    segments.try_reserve_exact(regions.len()).expect("MCFG segment storage unavailable");
    let mut budget = Budget::new();
    for region in regions.iter() {
        let segment = PcieSegmentGroup::try_new(
            region.segment,
            PAddr::from(region.base),
            region.start,
            region.end,
            &mut budget,
        )
        .unwrap_or_else(|error| panic!("PCI boot enumeration rejected: {error:?}"));
        segments.push(segment);
    }
    segments
}
