//! Serialized controls use the real charged node allocator/pool. Published
//! production records are never evicted; private fixture tables release only
//! their own records after leaving publication/accounting guards.
use alloc::vec::Vec;

use super::*;
use crate::memory::{
    self,
    AddressSpace,
    AddressSpaceRegistrationError,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

pub(super) fn run() {
    rejected_storage();
    publication_and_duplicate();
    pressure();
    abandoned();
    crate::logln!(
        "[principal history admission] charged candidates/publications, allocation rejection, \
         full ordinary/total pools, trusted-constructor reserve, close progress/recovery and \
         guarded abandonment passed"
    );
}

fn rejected_storage() {
    let before = admission::snapshot();
    for _ in 0..64 {
        assert!(PreparingHistory::try_new_with(false, |_| Err(AllocError)).is_err());
        assert_eq!(admission::snapshot(), before);
    }
}

fn publication_and_duplicate() {
    let before = admission::snapshot();
    let bytes = Storage::allocation_bytes() as u64;
    let mut first = PreparingHistory::try_new(false).unwrap();
    let mut duplicate = PreparingHistory::try_new(true).unwrap();
    let mut table = Table::new();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        admission::with_locked(|| {
            first.publish_into(&mut table, 1);
            table.get_mut(&1).unwrap().heap_bytes = 8192;
            duplicate.publish_into(&mut table, 1);
            assert!(first.0.is_none() && duplicate.0.is_some());
            assert_eq!(table.get(&1).unwrap().heap_bytes, 8192);
        });
    }
    let charged = admission::snapshot();
    assert_eq!(charged.0, [before.0[0] + 2 * bytes, before.0[1] + 2]);
    assert_eq!(charged.1, [before.1[0] + bytes, before.1[1] + 1]);
    first.cancel_unpublished();
    assert_eq!(admission::snapshot(), charged);
    duplicate.cancel_unpublished();
    assert_eq!(admission::snapshot().1, [before.1[0] + bytes, before.1[1] + 1]);
    table.take(&1).unwrap().release();
    assert_eq!(admission::snapshot(), before);
}

fn pressure() {
    let before = admission::snapshot();
    let old = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    let mut ordinary = Vec::with_capacity(4096);
    while let Ok(node) = PreparingHistory::try_new(false) {
        ordinary.push(node);
        assert!(ordinary.len() <= 4096);
    }
    let full = admission::snapshot();
    assert_eq!(full.1[1], 3072);
    for _ in 0..64 {
        assert!(
            PreparingHistory::try_new_with(false, |_| panic!("denied history reached allocator"))
                .is_err()
        );
    }
    let roots = memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count();
    let frames = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert_eq!(
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()),
        Err(AddressSpaceRegistrationError::DomainMetadataAllocationFailed)
    );
    assert_eq!(
        memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count(),
        roots
    );
    assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
    // Only the trusted architecture constructor selects reserve classification.
    let trusted =
        memory::register_user_address_space(AddressSpace::try_new_platform_user().unwrap())
            .unwrap();
    assert_eq!(admission::snapshot().1, full.1);
    assert_eq!(admission::snapshot().0[1], full.0[1] + 1);
    memory::close_user_address_space_handle(trusted).unwrap();
    assert_eq!(admission::snapshot(), full);
    let mut reserve = Vec::with_capacity(4096);
    while let Ok(node) = PreparingHistory::try_new(true) {
        reserve.push(node);
        assert!(reserve.len() <= 4096);
    }
    assert_eq!(admission::snapshot().0[1], 4096);
    assert!(
        PreparingHistory::try_new_with(true, |_| panic!("full total pool reached allocator"))
            .is_err()
    );
    // Teardown uses its existing metadata. It needs no new history allocation
    // and returns admission only after exact-root invalidation and node disposal.
    memory::close_user_address_space_handle(old).unwrap();
    let recovered =
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    memory::close_user_address_space_handle(recovered).unwrap();
    let ordinary_count = ordinary.len();
    let reserve_count = reserve.len();
    for node in reserve {
        node.cancel_unpublished();
    }
    for node in ordinary {
        node.cancel_unpublished();
    }
    assert_eq!(admission::snapshot(), before);
    crate::logln!(
        "[principal history pressure] {} ordinary + {} reserved real candidate nodes; ordinary \
         registration rejected before publication, trusted root admitted, total exhaustion and \
         existing-root close/recovery passed",
        ordinary_count,
        reserve_count
    );
}

fn abandoned() {
    let candidate = PreparingHistory::try_new(false).unwrap();
    let mut published = PreparingHistory::try_new(false).unwrap();
    let mut table = Table::new();
    published.publish_into(&mut table, 1);
    published.cancel_unpublished();
    let before = admission::snapshot();
    {
        let _history = HISTORY.lock();
        let _lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let _roots = memory::ADDRESS_SPACE_TABLE.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        let frames = memory::PHYSICAL_FRAME_ALLOCATOR.lock();
        let free = frames.free_frames();
        admission::with_locked(|| {
            drop(candidate);
            drop(table);
        });
        assert_eq!(frames.free_frames(), free);
    }
    assert_eq!(admission::snapshot(), before);
    // The unused candidate and published private fixture node remain charged.
    // No scalar recovery/re-adoption path exists for either abandoned owner.
}
