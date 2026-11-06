//! Requested-byte/count admission for the five global namespace map layouts.
//! Original classification follows prepared, published and detached storage
//! through actual node deallocation; logical retirement does not refund it.
use core::alloc::AllocError;

use crate::klib::{
    admission::{
        self,
        NamespaceNodes,
    },
    collections::retirement_list::{
        AdmittedMap,
        PreparedEntry,
        RetiredEntry,
    },
};

pub(crate) type Map<K, V> = AdmittedMap<K, V, Charge>;
pub(crate) type Prepared<T> = PreparedEntry<T, Charge>;
pub(crate) type Retired<T> = RetiredEntry<T, Charge>;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const NODE_LIMIT: [u64; 2] = crate::resource_policy::POLICY.namespace_nodes.total;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) const ORDINARY_LIMIT: [u64; 2] = crate::resource_policy::POLICY.namespace_nodes.ordinary;

pub(crate) type Charge =
    admission::NodeCharge<NamespaceNodes, crate::memory::kernel_buffers::MetadataSponsor>;

pub(crate) fn prepare<T>(
    storage: impl Into<crate::memory::kernel_buffers::CapturedStorage>,
) -> Result<Prepared<T>, AllocError> {
    prepare_with(storage, PreparedEntry::try_new_charged)
}
fn prepare_with<T>(
    storage: impl Into<crate::memory::kernel_buffers::CapturedStorage>,
    allocate: impl FnOnce(Charge) -> Result<Prepared<T>, AllocError>,
) -> Result<Prepared<T>, AllocError> {
    let storage = storage.into();
    admission::prepare_node_with_sponsor::<_, NamespaceNodes, _>(
        storage.platform(),
        PreparedEntry::<T, Charge>::allocation_bytes(),
        storage.metadata_sponsor(),
        allocate,
    )
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<NamespaceNodes, 2>()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<NamespaceNodes, 2, _>(work)
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn test_allocation<T>(family: &'static str) {
    let before = snapshot();
    for _ in 0..64 {
        assert!(prepare_with::<T>(false, |_| Err(AllocError)).is_err());
        assert_eq!(snapshot(), before);
    }
    crate::logln!(
        "[namespace node allocation] family={} bytes={}; 64 allocator rejections passed",
        family,
        Prepared::<T>::allocation_bytes()
    );
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn test_byte_denial() {
    let before = snapshot();
    assert!(
        prepare_with::<[u8; ORDINARY_LIMIT[0] as usize]>(false, |_| panic!(
            "ordinary byte denial reached allocator"
        ))
        .is_err()
    );
    assert!(
        prepare_with::<[u8; NODE_LIMIT[0] as usize]>(true, |_| panic!(
            "total byte denial reached allocator"
        ))
        .is_err()
    );
    assert_eq!(snapshot(), before);
}
