//! Independent backing for the three fixed-layout namespace account types.
//! Resource entry/ring/watch admission stays separate. Original backing class
//! follows the account Arc and private holder through final weak/allocator release.
use alloc::sync::Arc;
use core::alloc::AllocError;

use crate::klib::{
    admission::{
        self,
        CompletionAccounts,
    },
    charged_allocator::ChargedAllocator,
};

pub(crate) type AccountRef<T> = Arc<T, ChargedAllocator<Charge>>;

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const NODE_LIMIT: u64 = crate::resource_policy::POLICY.completion_accounts.total[0];
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY_LIMIT: u64 =
    crate::resource_policy::POLICY.completion_accounts.ordinary[0];

pub(crate) type Charge = admission::BundleCharge<CompletionAccounts>;

/// Only record, CQ and watch account constructors use this fixed-bundle pool.
pub(super) fn allocator(platform: bool) -> Result<ChargedAllocator<Charge>, AllocError> {
    ChargedAllocator::try_new(Charge::reserve(platform)?)
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
