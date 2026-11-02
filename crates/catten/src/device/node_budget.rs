//! Original backing admission for device namespace and capability nodes.
//! Candidate, published and detached nodes refund only after deallocation.
use core::alloc::AllocError;

use crate::klib::{
    admission::{
        self,
        DeviceNodes,
    },
    collections::retirement_list::{
        AdmittedMap,
        PreparedEntry,
        RetiredEntry,
    },
};

pub(super) type Map<K, V> = AdmittedMap<K, V, Charge>;
pub(super) type Prepared<T> = PreparedEntry<T, Charge>;
pub(super) type Retired<T> = RetiredEntry<T, Charge>;
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const LIMIT: [u64; 2] = crate::resource_policy::POLICY.device_nodes.total;
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const ORDINARY: [u64; 2] = crate::resource_policy::POLICY.device_nodes.ordinary;

pub(super) type Charge = admission::NodeCharge<DeviceNodes>;

pub(super) fn prepare<T>(platform: bool) -> Result<Prepared<T>, AllocError> {
    prepare_with(platform, PreparedEntry::try_new_charged)
}
fn prepare_with<T>(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<Prepared<T>, AllocError>,
) -> Result<Prepared<T>, AllocError> {
    admission::prepare_node_with::<_, DeviceNodes>(
        platform,
        PreparedEntry::<T, Charge>::allocation_bytes(),
        allocate,
    )
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<DeviceNodes, 2>()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn with_locked<T>(work: impl FnOnce() -> T) -> T {
    admission::with_locked::<DeviceNodes, 2, _>(work)
}
#[cfg(feature = "kernel_tests")]
mod tests;
#[cfg(feature = "kernel_tests")]
pub(super) use tests::run as test_admission;
