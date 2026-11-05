//! # PCI Express Enhanced Configuration Access Mechanism (ECAM)

pub mod capabilities;
pub mod headers;
pub mod pcie;

use super::topology::enumeration::ECAM_BYTES as ECAM_SIZE;
use crate::{
    cpu::isa::interface::memory::address::Address,
    logln,
    memory::{
        AddressSpaceInterface,
        KERNEL_AS,
        PAddr,
        VAddr,
        allocators::memory::PageSize,
        linear::{
            MemoryMapping,
            address_map::{
                LA_MAP,
                LinearMemoryRegion,
                RegionType,
            },
        },
    },
};

pub(super) fn map_ecam(base: PAddr) -> VAddr {
    logln!("[drivers::busses::pci_express] Mapping PCIe ECAM at physical address {:?}", base);
    let mut kas = KERNEL_AS.lock();
    logln!(
        "[drivers::busses::pci_express] Finding free virtual address range for PCIe ECAM mapping \
         of size {:?} bytes",
        ECAM_SIZE
    );
    let vbase = kas
        .find_free_region_large_aligned(
            ECAM_SIZE / PageSize::Large.num_bytes(),
            <LinearMemoryRegion as Into<(VAddr, VAddr)>>::into(
                *LA_MAP.get_region(RegionType::KernelMmio),
            ),
        )
        .expect("Failed to find free virtual address range for PCIe ECAM mapping");
    logln!(
        "[drivers::busses::pci_express] Mapping PCIe ECAM at physical address {:?} to virtual \
         address {:?}",
        base,
        vbase
    );
    let mut mem_mapping: MemoryMapping;
    let large_aligned = base.is_aligned_to(PageSize::Large.num_bytes());
    let step = if large_aligned {
        PageSize::Large.num_bytes()
    } else {
        PageSize::Standard.num_bytes()
    };
    for offset in (0..ECAM_SIZE).step_by(step) {
        mem_mapping = MemoryMapping {
            vaddr: vbase + offset,
            paddr: base + offset,
            page_type: crate::memory::linear::PageType::Mmio,
        };
        if large_aligned {
            kas.map_large_page(mem_mapping).expect("Failed to map large page for a PCIe ECAM!");
        } else {
            // Our MCFG validation accepts 1 MiB-aligned bus-zero bases. Preserve
            // the complete boot window when 2 MiB leaves cannot encode its base.
            kas.map_page(mem_mapping).expect("Failed to map page for a PCIe ECAM!");
        }
    }
    logln!(
        "[drivers::busses::pci_express] Successfully mapped PCIe ECAM at physical address {:?} to \
         virtual address {:?}",
        base,
        vbase
    );
    vbase
}
