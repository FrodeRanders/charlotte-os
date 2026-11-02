//! Backing admission for domain cells, persistent requester fences and contexts.
//! Original classification follows each node through actual deallocation.
use core::alloc::AllocError;

use crate::klib::{
    admission::{
        self,
        IommuNodes,
    },
    collections::retirement_list::{
        AdmittedMap,
        PreparedEntry,
        RetiredEntry,
    },
};

pub(in crate::device) type Map<K, V> = AdmittedMap<K, V, Charge>;
pub(super) type Prepared<T> = PreparedEntry<T, Charge>;
pub(super) type Retired<T> = RetiredEntry<T, Charge>;
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const LIMIT: [u64; 2] = crate::resource_policy::POLICY.iommu_nodes.total;
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const ORDINARY: [u64; 2] = crate::resource_policy::POLICY.iommu_nodes.ordinary;

pub(in crate::device) type Charge = admission::NodeCharge<IommuNodes>;

pub(super) fn prepare<T>(platform: bool) -> Result<Prepared<T>, AllocError> {
    prepare_with(platform, PreparedEntry::try_new_charged)
}
fn prepare_with<T>(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<Prepared<T>, AllocError>,
) -> Result<Prepared<T>, AllocError> {
    admission::prepare_node_with::<_, IommuNodes>(
        platform,
        PreparedEntry::<T, Charge>::allocation_bytes(),
        allocate,
    )
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::device) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<IommuNodes, 2>()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::device) fn with_locked<T>(work: impl FnOnce() -> T) -> T {
    admission::with_locked::<IommuNodes, 2, _>(work)
}
#[cfg(feature = "kernel_tests")]
mod tests;
#[cfg(feature = "kernel_tests")]
pub(in crate::device) use tests::{
    run as test_admission,
    with_ordinary_pressure,
};
