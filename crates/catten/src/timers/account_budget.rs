//! Backing admission for scheduler and completion timer-account bundles.
//! Original classification follows the account Arc and private allocator holder
//! through final strong/weak/allocator release, independently of event admission.
use core::alloc::AllocError;

use crate::klib::admission::{
    self,
    TimerAccounts,
};

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const NODE_LIMIT: u64 = crate::resource_policy::POLICY.timer_accounts.total[0];
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY_LIMIT: u64 = crate::resource_policy::POLICY.timer_accounts.ordinary[0];

pub(crate) type Charge = admission::BundleCharge<TimerAccounts>;

pub(super) fn reserve(platform: bool) -> Result<Charge, AllocError> {
    Charge::reserve(platform)
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn snapshot() -> (u64, u64) {
    let (total, ordinary) = admission::used::<TimerAccounts, 1>();
    (total[0], ordinary[0])
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<TimerAccounts, 1, _>(work)
}
