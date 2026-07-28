//! Minimal, bounds-checked ACPI IORT discovery for Arm SMMUv3.
//!
//! CharlotteOS currently supports PCI root-complex mappings to one SMMUv3
//! node. The parser deliberately preserves the requester-ID translation
//! instead of assuming that PCI BDF and SMMU StreamID are identical.

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

const IORT_HEADER_SIZE: usize = size_of::<SdtHeader>() + 12;
const NODE_HEADER_SIZE: usize = 16;
const NODE_SMMUV3: u8 = 4;
const NODE_PCI_ROOT_COMPLEX: u8 = 2;
const ID_MAPPING_SIZE: usize = 20;

#[derive(Debug, Clone, Copy)]
pub struct SmmuV3Config {
    pub base: usize,
    pub event_intid: u32,
    pub pri_intid: u32,
    pub gerror_intid: u32,
    pub sync_intid: u32,
    pub coherent: bool,
    input_base: u32,
    id_count: u32,
    output_base: u32,
}

impl SmmuV3Config {
    /// Translate a PCI requester ID through the root-complex IORT mapping.
    pub fn stream_id(self, requester_id: u32) -> Option<u32> {
        let offset = requester_id.checked_sub(self.input_base)?;
        if offset > self.id_count {
            return None;
        }
        self.output_base.checked_add(offset)
    }
}

#[derive(Clone, Copy)]
struct Node {
    offset: usize,
    kind: u8,
    length: usize,
    mapping_count: usize,
    mapping_offset: usize,
}

fn read_u8(base: *const u8, offset: usize) -> u8 {
    unsafe { ptr::read_unaligned(base.add(offset)) }
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

fn range_valid(offset: usize, length: usize, table_len: usize) -> bool {
    offset.checked_add(length).is_some_and(|end| end <= table_len)
}

/// Discover the first PCI root complex routed through an SMMUv3.
pub fn discover_smmuv3() -> Option<SmmuV3Config> {
    let table = *acpi::find_table_type(AcpiTableType::IORT).ok()?.first()?;
    let base = unsafe { table.into_hhdm_ptr::<u8>() };
    let header = unsafe { &*base.cast::<SdtHeader>() };
    if !header.validate() {
        return None;
    }
    let table_len = header.length as usize;
    if table_len < IORT_HEADER_SIZE {
        return None;
    }
    let node_count = read_u32(base, size_of::<SdtHeader>()) as usize;
    let mut offset = read_u32(base, size_of::<SdtHeader>() + 4) as usize;
    let mut nodes = alloc::vec::Vec::with_capacity(node_count);
    for _ in 0..node_count {
        if !range_valid(offset, NODE_HEADER_SIZE, table_len) {
            return None;
        }
        let length = read_u16(base, offset + 1) as usize;
        if length < NODE_HEADER_SIZE || !range_valid(offset, length, table_len) {
            return None;
        }
        nodes.push(Node {
            offset,
            kind: read_u8(base, offset),
            length,
            mapping_count: read_u32(base, offset + 8) as usize,
            mapping_offset: read_u32(base, offset + 12) as usize,
        });
        offset = offset.checked_add(length)?;
    }

    for root in nodes.iter().filter(|node| node.kind == NODE_PCI_ROOT_COMPLEX) {
        let mappings_start = root.offset.checked_add(root.mapping_offset)?;
        for index in 0..root.mapping_count {
            let mapping = mappings_start.checked_add(index.checked_mul(ID_MAPPING_SIZE)?)?;
            if !range_valid(mapping, ID_MAPPING_SIZE, root.offset + root.length) {
                return None;
            }
            let output_reference = read_u32(base, mapping + 12) as usize;
            let Some(smmu) = nodes
                .iter()
                .find(|node| node.offset == output_reference && node.kind == NODE_SMMUV3)
            else {
                continue;
            };
            // ACPI IORT SMMUv3 node fields through the GSIVs occupy 52 bytes
            // after the common node header.
            if smmu.length < 68 {
                return None;
            }
            return Some(SmmuV3Config {
                base: usize::try_from(read_u64(base, smmu.offset + 16)).ok()?,
                coherent: read_u32(base, smmu.offset + 24) & 1 != 0,
                event_intid: read_u32(base, smmu.offset + 44),
                pri_intid: read_u32(base, smmu.offset + 48),
                gerror_intid: read_u32(base, smmu.offset + 52),
                sync_intid: read_u32(base, smmu.offset + 56),
                input_base: read_u32(base, mapping),
                id_count: read_u32(base, mapping + 4),
                output_base: read_u32(base, mapping + 8),
            });
        }
    }
    None
}
