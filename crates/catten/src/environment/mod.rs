//! # Firmware Abstraction Layer
//!
//! Bootloader inputs and firmware-provided hardware descriptions.
//! The bootloader handles boot-time firmware interaction; the kernel reads
//! the resulting Limine responses and ACPI tables here. Firmware requirements
//! are described in [`boot_protocol`].
//!
//! ACPI supplies the PCIe segment descriptions used by device discovery.
//! Device-tree discovery, the SMC module and UEFI runtime services remain
//! unfinished.

use alloc::vec::Vec;

use crate::device_management::drivers::busses::pci_express::topology::PcieSegmentGroup;

// Advanced Configuration and Power Interface (ACPI)
#[cfg(any(target_arch = "x86_64", feature = "acpi"))]
pub mod acpi;
#[cfg(target_arch = "aarch64")]
pub mod arm_smc;
pub mod boot_protocol;
#[cfg(all(not(target_arch = "x86_64"), feature = "devicetree"))]
pub mod devicetree;
// Unified Extensible Firmware Interface (UEFI) Runtime Services
mod uefi_rts;

pub fn get_pcie_segment_groups() -> Vec<PcieSegmentGroup> {
    #[cfg(all(feature = "acpi", feature = "devicetree"))]
    compile_error!(
        "The Catten Kernel does not support compiling in both the acpi and devicetree
    features as standards do not allow systems to expose both at the same time. Please
    recompile your kernel with only the one you actually intend to use."
    );

    #[cfg(feature = "acpi")]
    {
        acpi::sdt::mcfg::parse_mcfg()
    }

    #[cfg(feature = "devicetree")]
    {
        todo!("Develop a way to get the information for each PCIe segment from a Device Tree.");
    }

    #[cfg(not(any(feature = "acpi", feature = "devicetree")))]
    {
        panic!(
            "The Catten Kernel can not function without either the acpi or devicetree features \
             enabled at compile time."
        );
    }
}
