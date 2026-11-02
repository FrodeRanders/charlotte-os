//! Requested memory-capability-node layouts share admission through actual deallocation.
use core::alloc::AllocError;

use crate::klib::{
    admission::{
        self,
        MemoryCapabilityNodes,
    },
    collections::retirement_list::PreparedEntry,
};

#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const TOTAL_LIMIT: [u64; 2] =
    crate::resource_policy::POLICY.memory_capability_nodes.total;
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const ORDINARY_LIMIT: [u64; 2] =
    crate::resource_policy::POLICY.memory_capability_nodes.ordinary;

pub(in crate::memory::object) type Charge = admission::NodeCharge<MemoryCapabilityNodes>;

pub(in crate::memory::object) fn prepare<T>(
    platform: bool,
) -> Result<PreparedEntry<T, Charge>, AllocError> {
    prepare_with(platform, PreparedEntry::try_new_charged)
}

pub(super) fn prepare_with<T>(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<PreparedEntry<T, Charge>, AllocError>,
) -> Result<PreparedEntry<T, Charge>, AllocError> {
    admission::prepare_node_with::<_, MemoryCapabilityNodes>(
        platform,
        PreparedEntry::<T, Charge>::allocation_bytes(),
        allocate,
    )
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory::object) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<MemoryCapabilityNodes, 2>()
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory::object) fn with_locked<T>(work: impl FnOnce() -> T) -> T {
    admission::with_locked::<MemoryCapabilityNodes, 2, _>(work)
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory::object) mod tests;
