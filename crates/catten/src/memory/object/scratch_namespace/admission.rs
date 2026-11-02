//! Requested scratch-node layouts share admission through actual deallocation.
use core::alloc::AllocError;

use crate::klib::{
    admission::{
        self,
        ScratchNodes,
    },
    collections::retirement_list::PreparedEntry,
};

#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const TOTAL_LIMIT: [u64; 2] = crate::resource_policy::POLICY.scratch_nodes.total;
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const ORDINARY_LIMIT: [u64; 2] = crate::resource_policy::POLICY.scratch_nodes.ordinary;

pub(crate) type Charge = admission::NodeCharge<ScratchNodes>;

pub(super) fn prepare<T>(platform: bool) -> Result<PreparedEntry<T, Charge>, AllocError> {
    prepare_with(platform, PreparedEntry::try_new_charged)
}

pub(super) fn prepare_with<T>(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<PreparedEntry<T, Charge>, AllocError>,
) -> Result<PreparedEntry<T, Charge>, AllocError> {
    admission::prepare_node_with::<_, ScratchNodes>(
        platform,
        PreparedEntry::<T, Charge>::allocation_bytes(),
        allocate,
    )
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<ScratchNodes, 2>()
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<ScratchNodes, 2, _>(work)
}
