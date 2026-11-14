//! Independent backing admission for close/readiness source disposal nodes.
//! Original bytes/class follow preparation and producer-LP queued storage.
use core::alloc::AllocError;

use super::PreparedDisposal;
use crate::{
    klib::admission::{
        self,
        DeferredNodes,
    },
    memory::kernel_buffers::{
        CapturedStorage,
        MetadataSponsor,
    },
};

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const LIMIT: [u64; 2] = crate::resource_policy::POLICY.deferred_nodes.total;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY: [u64; 2] = crate::resource_policy::POLICY.deferred_nodes.ordinary;

pub(crate) type Charge = admission::NodeCharge<DeferredNodes, MetadataSponsor>;

#[cfg(any(test, feature = "kernel_tests"))]
fn reserve(bytes: u64, platform: bool) -> Result<Charge, AllocError> {
    Charge::with_sponsor(
        platform,
        bytes,
        CapturedStorage::unidentified(platform).metadata_sponsor(),
    )
}
pub(crate) fn prepare(platform: bool) -> Result<PreparedDisposal, AllocError> {
    // IPC claim and DMA readiness callers still capture only classification.
    // Their original-principal propagation remains a separate Q4 boundary.
    prepare_funded(CapturedStorage::unidentified(platform))
}
pub(crate) fn prepare_funded(storage: CapturedStorage) -> Result<PreparedDisposal, AllocError> {
    prepare_funded_with(storage, PreparedDisposal::try_new_charged)
}
fn prepare_funded_with(
    storage: CapturedStorage,
    allocate: impl FnOnce(Charge) -> Result<PreparedDisposal, AllocError>,
) -> Result<PreparedDisposal, AllocError> {
    admission::prepare_node_with_sponsor::<_, DeferredNodes, _>(
        storage.platform(),
        PreparedDisposal::allocation_bytes(),
        storage.metadata_sponsor(),
        allocate,
    )
}
#[cfg(any(test, feature = "kernel_tests"))]
fn prepare_with(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<PreparedDisposal, AllocError>,
) -> Result<PreparedDisposal, AllocError> {
    prepare_funded_with(CapturedStorage::unidentified(platform), allocate)
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<DeferredNodes, 2>()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<DeferredNodes, 2, _>(work)
}
#[cfg(any(test, feature = "kernel_tests"))]
mod tests;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) use tests::funded_allocation_failure;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) use tests::test_source;
