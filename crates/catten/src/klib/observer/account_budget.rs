//! Independent admission for a fixed waiter-account allocation bundle: its Arc
//! and the private ChargedAllocator holder. Original classification follows both
//! allocations through the last strong/weak/allocator reference, not root exit.

use core::alloc::AllocError;

#[cfg(any(test, feature = "kernel_tests"))]
use crate::klib::admission;
use crate::klib::admission::WaiterAccounts;

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const NODE_LIMIT: u64 = crate::resource_policy::POLICY.waiter_accounts.total[0];
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY_LIMIT: u64 = crate::resource_policy::POLICY.waiter_accounts.ordinary[0];

pub(crate) type Allocator = crate::klib::account_backing::Allocator<
    WaiterAccounts,
    crate::memory::kernel_buffers::ReactorSponsor,
>;
pub(crate) type AccountRef<T> = alloc::sync::Arc<T, Allocator>;

pub(super) fn allocator(
    storage: impl Into<crate::memory::kernel_buffers::CapturedStorage>,
) -> Result<Allocator, AllocError> {
    let storage = storage.into();
    crate::klib::account_backing::allocator(storage.platform(), storage.reactor_sponsor())
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
