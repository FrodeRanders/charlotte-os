//! # PCI Express Enhanced Configuration Access Mechanism (ECAM)
pub mod headers;
pub mod pci_local;
pub mod pcie;

use super::Error;
use super::pcie::{
    PcieBusSegmentNum,
    PcieDeviceNum,
    PcieFunctionNum,
    PcieSegmentGroup,
    PcieSegmentGroupNum,
};
use crate::device_manager::DEVICE_TOPOLOGY;
use crate::drivers::busses::pci::ecam::pci_local::PcieCfgSpace;
use crate::drivers::busses::pci::{MAX_DEVICES_PER_BUS, MAX_FUNCTIONS_PER_DEVICE};
use crate::klib::size::mebibytes;
use crate::logln;
use crate::memory::allocators::memory::PageSize;
use crate::memory::linear::MemoryMapping;
use crate::memory::linear::address_map::{LA_MAP, LinearMemoryRegion, RegionType};
use crate::memory::{AddressSpaceInterface, KERNEL_AS, PAddr, VAddr};

const ECAM_SIZE: usize = mebibytes(256); /* Each PCIe segment group's ECAM occupies 256 MiB of address space */

pub(super) fn map_ecam(base: PAddr) -> VAddr {
    logln!("[drivers::bus::pci] Mapping PCIe ECAM at physical address {:?}", base);
    let mut kas = KERNEL_AS.lock();
    logln!(
        "[drivers::bus::pci] Finding free virtual address range for PCIe ECAM mapping of size \
         {:?} bytes",
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
        "[drivers::bus::pci] Mapping PCIe ECAM at physical address {:?} to virtual address {:?}",
        base,
        vbase
    );
    let mut mem_mapping: MemoryMapping;
    for offset in (0..ECAM_SIZE).step_by(mebibytes(2)) {
        mem_mapping = MemoryMapping {
            vaddr: vbase + offset,
            paddr: base + offset,
            page_type: crate::memory::linear::PageType::Mmio,
        };
        kas.map_large_page(mem_mapping);
    }
    logln!(
        "[drivers::bus::pci] Successfully mapped PCIe ECAM at physical address {:?} to virtual \
         address {:?}",
        base,
        vbase
    );
    vbase
}

type DeviceSlice = [PcieCfgSpace; MAX_FUNCTIONS_PER_DEVICE]; /* Each bus segment contains up to 32 devices, and each device contains up to 8 functions, so each bus segment can be represented as a slice of 256 PcieCfgSpace structs (32 devices * 8 functions) */

pub struct Ecam {
    bus_segment_slices: [DeviceSlice; MAX_DEVICES_PER_BUS],
}
