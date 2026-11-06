//! Backing admission for scheduler and completion timer-account bundles.
//! Original classification follows the account Arc and private allocator holder
//! through final strong/weak/allocator release, independently of event admission.
use core::alloc::AllocError;

#[cfg(any(test, feature = "kernel_tests"))]
use crate::klib::admission;
use crate::klib::admission::TimerAccounts;

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const NODE_LIMIT: u64 = crate::resource_policy::POLICY.timer_accounts.total[0];
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY_LIMIT: u64 = crate::resource_policy::POLICY.timer_accounts.ordinary[0];

pub(crate) type Allocator = crate::klib::account_backing::Allocator<
    TimerAccounts,
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
    let (total, ordinary) = admission::used::<TimerAccounts, 1>();
    (total[0], ordinary[0])
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<TimerAccounts, 1, _>(work)
}
