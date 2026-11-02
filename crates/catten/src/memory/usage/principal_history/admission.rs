//! One charge follows each candidate/published history node through actual
//! allocation lifetime. Ordinary work cannot consume the reserved quarter.
use core::alloc::AllocError;

use crate::klib::admission::{
    self,
    PrincipalHistory,
};

pub(in crate::memory) type Charge = admission::NodeCharge<PrincipalHistory>;

pub(super) fn reserve(platform: bool, bytes: usize) -> Result<Charge, AllocError> {
    Charge::reserve(platform, u64::try_from(bytes).map_err(|_| AllocError)?)
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<PrincipalHistory, 2>()
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<PrincipalHistory, 2, _>(work)
}
