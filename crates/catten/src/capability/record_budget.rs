//! Authority node backing, independent of published capability admission.
//! Candidates and detached nodes retain their original byte/count reservation.
use core::alloc::AllocError;

use super::{
    AllocationError,
    Entry,
    ObjectCapability,
};
use crate::klib::{
    admission::{
        self,
        AuthorityNodes,
    },
    collections::retirement_list::{
        AdmittedMap,
        PreparedEntry,
        RetiredEntry,
    },
};

pub(super) type Map<K, V> = AdmittedMap<K, V, Charge>;
pub(super) type Prepared = PreparedEntry<(ObjectCapability, Entry), Charge>;
pub(super) type Retired = RetiredEntry<(ObjectCapability, Entry), Charge>;
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const LIMIT: [u64; 2] = crate::resource_policy::POLICY.authority_nodes.total;
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) const ORDINARY: [u64; 2] = crate::resource_policy::POLICY.authority_nodes.ordinary;

pub(super) type Charge =
    admission::NodeCharge<AuthorityNodes, crate::memory::kernel_buffers::MetadataSponsor>;

fn reserve(
    bytes: u64,
    storage: impl Into<crate::memory::kernel_buffers::CapturedStorage>,
) -> Result<Charge, AllocationError> {
    let storage = storage.into();
    Charge::with_sponsor(storage.platform(), bytes, storage.metadata_sponsor())
        .map_err(|_| AllocationError::ResourceLimit)
}
pub(super) fn prepare(
    storage: impl Into<crate::memory::kernel_buffers::CapturedStorage>,
) -> Result<Prepared, AllocationError> {
    prepare_with(storage.into(), Prepared::try_new_charged)
}
fn prepare_with(
    storage: crate::memory::kernel_buffers::CapturedStorage,
    allocate: impl FnOnce(Charge) -> Result<Prepared, AllocError>,
) -> Result<Prepared, AllocationError> {
    let bytes =
        u64::try_from(Prepared::allocation_bytes()).map_err(|_| AllocationError::ResourceLimit)?;
    let charge = reserve(bytes, storage)?;
    allocate(charge).map_err(|_| AllocationError::AllocationFailed)
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<AuthorityNodes, 2>()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<AuthorityNodes, 2, _>(work)
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn test_rejection() {
    let before = snapshot();
    for _ in 0..64 {
        assert!(matches!(
            prepare_with(false.into(), |_| Err(AllocError)),
            Err(AllocationError::AllocationFailed)
        ));
        assert_eq!(snapshot(), before);
    }
    // The real layout is currently count-bound. Independently exercise byte
    // denial before invoking the allocator, including future layout growth.
    assert!(matches!(reserve(ORDINARY[0] + 1, false), Err(AllocationError::ResourceLimit)));
    assert!(matches!(reserve(LIMIT[0] + 1, true), Err(AllocationError::ResourceLimit)));
    assert_eq!(snapshot(), before);
}

#[cfg(feature = "kernel_tests")]
pub(super) fn test_principal_rejection(storage: crate::memory::kernel_buffers::CapturedStorage) {
    let before = snapshot();
    for _ in 0..64 {
        assert!(prepare_with(storage, |_| Err(AllocError)).is_err());
        assert_eq!(snapshot(), before);
    }
}
