//! x86_64 IOMMU dispatcher.
//!
//! A given machine exposes either Intel VT-d (DMAR ACPI table) or AMD-Vi
//! (IVRS ACPI table). This module selects the active backend at first use and
//! forwards every operation, so the device-capability layer stays agnostic to
//! the underlying vendor implementation.

pub use super::dma_common::{
    Direction,
    Error,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Backend {
    Vtd,
    AmdVi,
}

fn detect() -> Result<Backend, Error> {
    if crate::environment::acpi::sdt::dmar::discover_vtd().is_some() {
        Ok(Backend::Vtd)
    } else if crate::environment::acpi::sdt::ivrs::discover_amd_vi().is_some() {
        Ok(Backend::AmdVi)
    } else {
        Err(Error::Unsupported)
    }
}

pub fn initialize_early() -> Result<(), Error> {
    match detect()? {
        Backend::Vtd => super::vt_d::initialize_early(),
        Backend::AmdVi => super::amd_vi::initialize_early(),
    }
}

pub fn stream_id(requester_id: u32) -> Result<u32, Error> {
    match detect()? {
        Backend::Vtd => super::vt_d::stream_id(requester_id),
        Backend::AmdVi => super::amd_vi::stream_id(requester_id),
    }
}

pub fn create_domain(sid: u32, msi_address: Option<u64>) -> Result<u64, Error> {
    match detect()? {
        Backend::Vtd => super::vt_d::create_domain(sid, msi_address),
        Backend::AmdVi => super::amd_vi::create_domain(sid, msi_address),
    }
}

pub fn map(
    domain_id: u64,
    caller: crate::memory::AddressSpaceId,
    memory_cap: u64,
    direction: Direction,
    exclusive: bool,
) -> Result<u64, Error> {
    match detect()? {
        Backend::Vtd => super::vt_d::map(domain_id, caller, memory_cap, direction, exclusive),
        Backend::AmdVi => super::amd_vi::map(domain_id, caller, memory_cap, direction, exclusive),
    }
}

pub fn unmap(domain_id: u64, iova: u64) -> Result<(), Error> {
    match detect()? {
        Backend::Vtd => super::vt_d::unmap(domain_id, iova),
        Backend::AmdVi => super::amd_vi::unmap(domain_id, iova),
    }
}

pub fn destroy_domain(domain_id: u64) -> Result<(), Error> {
    match detect()? {
        Backend::Vtd => super::vt_d::destroy_domain(domain_id),
        Backend::AmdVi => super::amd_vi::destroy_domain(domain_id),
    }
}

pub fn handle_interrupt(intid: u32) -> bool {
    match detect() {
        Ok(Backend::Vtd) => super::vt_d::handle_interrupt(intid),
        Ok(Backend::AmdVi) => super::amd_vi::handle_interrupt(intid),
        Err(_) => false,
    }
}

pub fn fault_count() -> u64 {
    match detect() {
        Ok(Backend::Vtd) => super::vt_d::fault_count(),
        Ok(Backend::AmdVi) => super::amd_vi::fault_count(),
        Err(_) => 0,
    }
}

pub fn pending_fault_events() -> u32 {
    match detect() {
        Ok(Backend::Vtd) => super::vt_d::pending_fault_events(),
        Ok(Backend::AmdVi) => super::amd_vi::pending_fault_events(),
        Err(_) => 0,
    }
}
