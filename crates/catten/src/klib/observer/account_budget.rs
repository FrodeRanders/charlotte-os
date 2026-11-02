//! Independent admission for a fixed waiter-account allocation bundle: its Arc
//! and the private ChargedAllocator holder. Original classification follows both
//! allocations through the last strong/weak/allocator reference, not root exit.

use core::alloc::AllocError;

use crate::klib::admission::{
    self,
    WaiterAccounts,
};

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const NODE_LIMIT: u64 = crate::resource_policy::POLICY.waiter_accounts.total[0];
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY_LIMIT: u64 = crate::resource_policy::POLICY.waiter_accounts.ordinary[0];

pub(crate) type Charge = admission::BundleCharge<WaiterAccounts>;

pub(super) fn reserve(platform: bool) -> Result<Charge, AllocError> {
    Charge::reserve(platform)
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn snapshot() -> (u64, u64) {
    let (node, ordinary) = admission::used::<WaiterAccounts, 1>();
    (node[0], ordinary[0])
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<WaiterAccounts, 1, _>(work)
}

#[cfg(any(test, feature = "kernel_tests"))]
mod tests;

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn test_admission() {
    tests::run();
}
