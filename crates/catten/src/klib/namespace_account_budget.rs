//! Backing admission for capability, IPC endpoint/record and mailbox accounts.
//! Fixed account/holder bundles retain their original storage class through
//! actual strong/weak/allocator lifetime, independently of resource admission.
use alloc::sync::Arc;
use core::alloc::AllocError;

use crate::klib::{
    admission::{
        self,
        NamespaceAccounts,
    },
    charged_allocator::ChargedAllocator,
};

pub(crate) type AccountRef<T> = Arc<T, ChargedAllocator<Charge>>;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const NODE_LIMIT: u64 = crate::resource_policy::POLICY.namespace_accounts.total[0];
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY_LIMIT: u64 =
    crate::resource_policy::POLICY.namespace_accounts.ordinary[0];

pub(crate) type Charge = admission::BundleCharge<NamespaceAccounts>;

/// Only the four fixed-layout namespace account constructors use this pool.
pub(crate) fn allocator(platform: bool) -> Result<ChargedAllocator<Charge>, AllocError> {
    ChargedAllocator::try_new(Charge::reserve(platform)?)
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn snapshot() -> (u64, u64) {
    let (total, ordinary) = admission::used::<NamespaceAccounts, 1>();
    (total[0], ordinary[0])
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<NamespaceAccounts, 1, _>(work)
}
