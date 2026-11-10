//! Independent backing admission for completion, detached-operation and CQ
//! registry nodes, including unused candidates and detached/backlogged storage.
use core::alloc::AllocError;

use crate::{
    klib::{
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
    },
    memory::kernel_buffers::{
        CapturedStorage,
        MetadataSponsor,
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

pub(super) type Charge = admission::NodeCharge<CompletionNodes, MetadataSponsor>;

pub(super) fn prepare<T>(storage: impl Into<CapturedStorage>) -> Result<Prepared<T>, AllocError> {
    prepare_with(storage, PreparedEntry::try_new_charged)
}
fn prepare_with<T>(
    storage: impl Into<CapturedStorage>,
    allocate: impl FnOnce(Charge) -> Result<Prepared<T>, AllocError>,
) -> Result<Prepared<T>, AllocError> {
    let storage = storage.into();
    admission::prepare_node_with_sponsor::<_, CompletionNodes, _>(
        storage.platform(),
        PreparedEntry::<T, Charge>::allocation_bytes(),
        storage.metadata_sponsor(),
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
pub(super) use tests::{
    principal_root as test_principal_root,
    run as test_admission,
};
