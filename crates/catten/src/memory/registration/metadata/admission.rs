//! Root policy/usage/authority/memory-account admission follows allocation lifetime,
//! including partial preparation, detached metadata and terminal retention.
use core::alloc::AllocError;

use crate::klib::{
    admission::{
        self,
        RootMetadata,
    },
    collections::retirement_list::PreparedEntry,
};

pub(in crate::memory) type Charge =
    admission::NodeCharge<RootMetadata, crate::memory::kernel_buffers::MetadataSponsor>;

pub(in crate::memory) fn prepare<T>(
    storage: impl Into<crate::memory::kernel_buffers::CapturedStorage>,
) -> Result<PreparedEntry<T, Charge>, AllocError> {
    prepare_with(storage, PreparedEntry::try_new_charged)
}

pub(in crate::memory) fn prepare_with<T>(
    storage: impl Into<crate::memory::kernel_buffers::CapturedStorage>,
    allocate: impl FnOnce(Charge) -> Result<PreparedEntry<T, Charge>, AllocError>,
) -> Result<PreparedEntry<T, Charge>, AllocError> {
    let storage = storage.into();
    admission::prepare_node_with_sponsor::<_, RootMetadata, _>(
        storage.platform(),
        PreparedEntry::<T, Charge>::allocation_bytes(),
        storage.metadata_sponsor(),
        allocate,
    )
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<RootMetadata, 2>()
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<RootMetadata, 2, _>(work)
}
