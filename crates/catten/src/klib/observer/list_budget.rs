//! Independent admission for observer-list allocations, including empty lists
//! retained by registration tokens and weak-only backing. Entry limits account
//! for a different lifetime. No namespace lookup or allocator under this lock.

use crate::klib::admission::{
    self,
    ObserverLists,
};
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const NODE_LIMIT: usize =
    crate::resource_policy::POLICY.observer_lists.total[0] as usize;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY_LIMIT: usize =
    crate::resource_policy::POLICY.observer_lists.ordinary[0] as usize;

pub(crate) type Charge = admission::BundleCharge<ObserverLists>;
/// Classification comes from captured source admission or the first waiting
/// generation's sponsor, never an application-supplied role or ASID.
pub(crate) fn reserve(platform: bool) -> Result<Charge, super::registration::RegistrationError> {
    Charge::reserve(platform).map_err(|_| super::registration::RegistrationError::ResourceLimit)
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn node_used() -> (usize, usize) {
    let (total, ordinary) = admission::used::<ObserverLists, 1>();
    (total[0] as usize, ordinary[0] as usize)
}
