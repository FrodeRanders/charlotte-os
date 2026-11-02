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

pub(in crate::memory) type Charge = admission::NodeCharge<RootMetadata>;

pub(in crate::memory) fn prepare<T>(
    platform: bool,
) -> Result<PreparedEntry<T, Charge>, AllocError> {
    prepare_with(platform, PreparedEntry::try_new_charged)
}

pub(in crate::memory) fn prepare_with<T>(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<PreparedEntry<T, Charge>, AllocError>,
) -> Result<PreparedEntry<T, Charge>, AllocError> {
    admission::prepare_node_with::<_, RootMetadata>(
        platform,
        PreparedEntry::<T, Charge>::allocation_bytes(),
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
