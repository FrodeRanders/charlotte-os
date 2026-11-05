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

pub(super) struct Charge {
    _family: admission::Reservation<SnapshotBuffers, 2>,
    _principal: admission::ByteCharge<crate::memory::kernel_buffers::Sponsor>,
}
pub(super) fn reserve(
    storage: crate::memory::kernel_buffers::CapturedStorage,
    bytes: usize,
) -> Result<Charge, MemoryObjectError> {
    let amount = [u64::try_from(bytes).map_err(|_| MemoryObjectError::ResourceLimit)?, 1];
    let principal =
        storage.reserve_bytes(amount[0]).map_err(|_| MemoryObjectError::ResourceLimit)?;
    let family = admission::Reservation::reserve(storage.platform(), amount)
        .map_err(|_| MemoryObjectError::ResourceLimit)?;
    Ok(Charge {
        _family: family,
        _principal: principal,
    })
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<SnapshotBuffers, 2>()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn with_locked<T>(work: impl FnOnce() -> T) -> T {
    admission::with_locked::<SnapshotBuffers, 2, _>(work)
}
