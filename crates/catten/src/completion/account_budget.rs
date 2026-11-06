//! Independent backing for the three fixed-layout namespace account types.
//! Resource entry/ring/watch admission stays separate. Original backing class
//! follows the account Arc and private holder through final weak/allocator release.
use core::alloc::AllocError;

#[cfg(any(test, feature = "kernel_tests"))]
use crate::klib::admission;
use crate::klib::admission::CompletionAccounts;

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const NODE_LIMIT: u64 = crate::resource_policy::POLICY.completion_accounts.total[0];
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY_LIMIT: u64 =
    crate::resource_policy::POLICY.completion_accounts.ordinary[0];

pub(crate) type Allocator = crate::klib::account_backing::Allocator<
    CompletionAccounts,
    crate::memory::kernel_buffers::ReactorSponsor,
>;
pub(crate) type AccountRef<T> = alloc::sync::Arc<T, Allocator>;

/// Only record, CQ and watch account constructors use this fixed-bundle pool.
pub(super) fn allocator(
    storage: impl Into<crate::memory::kernel_buffers::CapturedStorage>,
) -> Result<Allocator, AllocError> {
    let storage = storage.into();
    crate::klib::account_backing::allocator(storage.platform(), storage.reactor_sponsor())
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn snapshot() -> (u64, u64) {
    let (total, ordinary) = admission::used::<CompletionAccounts, 1>();
    (total[0], ordinary[0])
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<CompletionAccounts, 1, _>(work)
}

#[cfg(any(test, feature = "kernel_tests"))]
mod tests;

#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn test_admission() {
    tests::run();
}
