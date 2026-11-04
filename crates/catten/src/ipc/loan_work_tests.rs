//! Common guard/size controls for the two complete loan-cleanup parents.
use super::*;
use crate::memory::{
    ADDRESS_SPACE_LIFECYCLE,
    ADDRESS_SPACE_TABLE,
    PHYSICAL_FRAME_ALLOCATOR,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
    object,
};

pub(super) fn bytes(count: usize) -> u64 {
    (count * core::mem::size_of::<(MemoryBorrow, object::LoanRevocation)>()) as u64
}

pub(super) fn with_guards(work: impl FnOnce()) {
    let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
    let _roots = ADDRESS_SPACE_TABLE.lock();
    let _ipc = IPC.write();
    let _heap = PRIMARY_ALLOCATOR.lock();
    let _physical = PHYSICAL_FRAME_ALLOCATOR.lock();
    object::retirement_tests::with_registry_locked(|| {
        crate::capability::admission_tests::test_with_registry_locked(|| {
            node_budget::with_locked(work);
        });
    });
}
