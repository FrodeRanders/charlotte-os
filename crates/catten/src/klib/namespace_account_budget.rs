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
    charged_allocator::{
        ChargedAllocator,
        PreAdmitted,
    },
};

pub(crate) type Allocator = ChargedAllocator<
    Charge,
    PreAdmitted,
    admission::SponsoredAllocator<crate::memory::kernel_buffers::NamespaceSponsor>,
>;
pub(crate) type AccountRef<T> = Arc<T, Allocator>;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const NODE_LIMIT: u64 = crate::resource_policy::POLICY.namespace_accounts.total[0];
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY_LIMIT: u64 =
    crate::resource_policy::POLICY.namespace_accounts.ordinary[0];

pub(crate) type Charge = admission::BundleCharge<NamespaceAccounts>;

/// Only the four fixed-layout namespace account constructors use this pool.
pub(crate) fn allocator(
    storage: impl Into<crate::memory::kernel_buffers::CapturedStorage>,
) -> Result<Allocator, AllocError> {
    let storage = storage.into();
    Allocator::try_new_with_backing(
        Charge::reserve(storage.platform())?,
        admission::SponsoredAllocator::new(storage.namespace_sponsor()),
    )
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
