//! Requested buffer bytes/counts, retained until actual buffer deallocation.
use super::*;
use crate::klib::admission::{
    self,
    SnapshotBuffers,
};

#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const TOTAL_LIMIT: [u64; 2] = crate::resource_policy::POLICY.snapshot_buffers.total;
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const ORDINARY_LIMIT: [u64; 2] =
    crate::resource_policy::POLICY.snapshot_buffers.ordinary;

pub(super) type Charge = admission::Reservation<SnapshotBuffers, 2>;
pub(super) fn reserve(platform: bool, bytes: usize) -> Result<Charge, MemoryObjectError> {
    let amount = [u64::try_from(bytes).map_err(|_| MemoryObjectError::ResourceLimit)?, 1];
    Charge::reserve(platform, amount).map_err(|_| MemoryObjectError::ResourceLimit)
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<SnapshotBuffers, 2>()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn with_locked<T>(work: impl FnOnce() -> T) -> T {
    admission::with_locked::<SnapshotBuffers, 2, _>(work)
}
