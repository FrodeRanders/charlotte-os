//! Mapping-node layout admission follows actual allocation lifetime, including
//! unused candidates and detached or abandoned retirement records.
use core::alloc::AllocError;

use super::*;
use crate::{
    klib::admission::{
        self,
        MappingNodes,
    },
    memory::kernel_buffers::{
        CapturedStorage,
        MetadataSponsor,
    },
};

#[cfg(any(test, feature = "kernel_tests"))]
const TOTAL_LIMIT: [u64; 2] = crate::resource_policy::POLICY.mapping_nodes.total;
#[cfg(any(test, feature = "kernel_tests"))]
const ORDINARY_LIMIT: [u64; 2] = crate::resource_policy::POLICY.mapping_nodes.ordinary;

pub(in crate::memory::object) type Charge = admission::NodeCharge<MappingNodes, MetadataSponsor>;

pub(super) fn prepare(storage: impl Into<CapturedStorage>) -> Result<Storage, MemoryObjectError> {
    prepare_with(storage, Storage::try_new_charged)
}
fn prepare_with(
    storage: impl Into<CapturedStorage>,
    allocate: impl FnOnce(Charge) -> Result<Storage, AllocError>,
) -> Result<Storage, MemoryObjectError> {
    let storage = storage.into();
    admission::prepare_node_with_sponsor::<_, MappingNodes, _>(
        storage.platform(),
        Storage::allocation_bytes(),
        storage.metadata_sponsor(),
        allocate,
    )
    .map_err(|_| MemoryObjectError::ResourceLimit)
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
