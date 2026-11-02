//! Independent backing admission for completion, detached-operation and CQ
//! registry nodes, including unused candidates and detached/backlogged storage.
use core::alloc::AllocError;

use crate::klib::{
    admission::{
        self,
        CompletionNodes,
    },
    collections::retirement_list::{
        AdmittedMap,
        PreparedEntry,
        RetiredEntry,
        RetirementList,
    },
};

pub(super) type Map<K, V> = AdmittedMap<K, V, Charge>;
pub(super) type Prepared<T> = PreparedEntry<T, Charge>;
pub(super) type Retired<T> = RetiredEntry<T, Charge>;
pub(super) type List<T> = RetirementList<T, Charge>;
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const LIMIT: [u64; 2] = crate::resource_policy::POLICY.completion_nodes.total;
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const ORDINARY: [u64; 2] = crate::resource_policy::POLICY.completion_nodes.ordinary;

pub(super) type Charge = admission::NodeCharge<CompletionNodes>;

pub(super) fn prepare<T>(platform: bool) -> Result<Prepared<T>, AllocError> {
    prepare_with(platform, PreparedEntry::try_new_charged)
}
fn prepare_with<T>(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<Prepared<T>, AllocError>,
) -> Result<Prepared<T>, AllocError> {
    admission::prepare_node_with::<_, CompletionNodes>(
        platform,
        PreparedEntry::<T, Charge>::allocation_bytes(),
        allocate,
    )
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<CompletionNodes, 2>()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<CompletionNodes, 2, _>(work)
}

#[cfg(feature = "kernel_tests")]
mod tests;
#[cfg(feature = "kernel_tests")]
pub(super) use tests::run as test_admission;
