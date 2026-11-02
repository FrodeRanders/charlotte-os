//! Uniform observer allocation ownership through final weak/allocator release.
//! Count admission covers the fixed private charge holder; actual Arc layout
//! bytes (including header/alignment) are admitted before payload allocation.
use core::alloc::{
    AllocError,
    Layout,
};

use crate::klib::{
    admission::{
        self,
        ObserverBacking,
    },
    charged_allocator::{
        ChargedAllocator,
        PayloadAdmission,
    },
};
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const LIMIT: [u64; 2] = crate::resource_policy::POLICY.observer_backing.total;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY: [u64; 2] = crate::resource_policy::POLICY.observer_backing.ordinary;

pub(crate) type Charge = admission::LayoutCharge<ObserverBacking>;

#[derive(Debug)]
pub(crate) struct LayoutAdmission;
impl PayloadAdmission<Charge> for LayoutAdmission {
    fn reserve(charge: &Charge, layout: Layout) -> Result<(), AllocError> {
        charge.admit_layout(layout)
    }

    fn rollback(charge: &Charge, layout: Layout) {
        charge.rollback_layout(layout);
    }
}
pub(crate) type Allocator = ChargedAllocator<Charge, LayoutAdmission>;
pub(super) fn allocator(platform: bool) -> Result<Allocator, AllocError> {
    Allocator::try_new_with_policy(Charge::reserve(platform)?)
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<ObserverBacking, 2>()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<ObserverBacking, 2, _>(work)
}

#[cfg(feature = "kernel_tests")]
#[path = "backing/tests.rs"]
mod tests;
#[cfg(feature = "kernel_tests")]
pub(crate) use tests::test_admission;
