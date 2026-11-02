//! Independent backing admission for close/readiness source disposal nodes.
//! Original bytes/class follow preparation and producer-LP queued storage.
use core::alloc::AllocError;

use super::PreparedDisposal;
use crate::klib::admission::{
    self,
    DeferredNodes,
};

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const LIMIT: [u64; 2] = crate::resource_policy::POLICY.deferred_nodes.total;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY: [u64; 2] = crate::resource_policy::POLICY.deferred_nodes.ordinary;

pub(crate) type Charge = admission::NodeCharge<DeferredNodes>;

fn reserve(bytes: u64, platform: bool) -> Result<Charge, AllocError> {
    Charge::reserve(platform, bytes)
}
pub(crate) fn prepare(platform: bool) -> Result<PreparedDisposal, AllocError> {
    prepare_with(platform, PreparedDisposal::try_new_charged)
}
fn prepare_with(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<PreparedDisposal, AllocError>,
) -> Result<PreparedDisposal, AllocError> {
    let bytes = u64::try_from(PreparedDisposal::allocation_bytes()).map_err(|_| AllocError)?;
    allocate(reserve(bytes, platform)?)
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
pub(crate) use tests::test_source;
