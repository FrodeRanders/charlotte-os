//! Mapping-node layout admission follows actual allocation lifetime, including
//! unused candidates and detached or abandoned retirement records.
use super::*;
use crate::klib::admission::{
    self,
    MappingNodes,
};

#[cfg(any(test, feature = "kernel_tests"))]
const TOTAL_LIMIT: [u64; 2] = crate::resource_policy::POLICY.mapping_nodes.total;
#[cfg(any(test, feature = "kernel_tests"))]
const ORDINARY_LIMIT: [u64; 2] = crate::resource_policy::POLICY.mapping_nodes.ordinary;

pub(in crate::memory::object) type Charge = admission::NodeCharge<MappingNodes>;

pub(super) fn prepare(platform: bool) -> Result<Storage, MemoryObjectError> {
    prepare_with(platform, |charge| {
        Storage::try_new_charged(charge).map_err(|_| MemoryObjectError::ResourceLimit)
    })
}
fn prepare_with(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<Storage, MemoryObjectError>,
) -> Result<Storage, MemoryObjectError> {
    allocate(
        Charge::reserve(platform, Storage::allocation_bytes() as u64)
            .map_err(|_| MemoryObjectError::ResourceLimit)?,
    )
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<MappingNodes, 2>()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn with_locked<T>(work: impl FnOnce() -> T) -> T {
    admission::with_locked::<MappingNodes, 2, _>(work)
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(super) mod tests;
