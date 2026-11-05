//! Serialized controls: real root/data frames, private principal ceilings and
//! counter-only family pressure. These are not saturation/progress acceptance.
use super::*;
use crate::{
    klib::admission::{
        self,
        Reservation,
    },
    memory::{
        self,
        AddressSpace,
        AddressSpaceHandle,
        AddressSpaceInterface,
        PreparingUserBacking,
        VAddr,
        backing_budget::Kind,
        kernel_buffers,
        linear::PageType,
    },
};

fn root(principal: u64) -> AddressSpace {
    AddressSpace::try_new_principal_user(
        false,
        memory::prepare_principal_backing(principal, false).unwrap(),
    )
    .unwrap()
}
fn domain(principal: u64) -> AddressSpaceHandle {
    let handle = memory::register_user_address_space(root(principal)).unwrap();
    memory::register_domain_authority(handle, principal, 0).unwrap();
    handle
}
fn install(space: &mut AddressSpace, index: usize) {
    PreparingUserBacking::new(space, Kind::Heap)
        .unwrap()
        .map_with(
            VAddr::from(charlotte_launch::HEAP_VADDR + 4096 * index),
            PageType::UserData,
            |space, mapping| space.map_existing_page(mapping).is_ok(),
        )
        .unwrap();
}
fn pool() -> ([u64; 2], [u64; 2]) {
    admission::used::<MemoryCapabilityNodes, 2>()
}

pub(crate) fn run() {
    let principal = charlotte_launch::artifact_principal_id(b"root-frame-buffer-principal");
    let other = charlotte_launch::artifact_principal_id(b"root-frame-buffer-other");
    let before = pool();
    let mut unpublished = root(principal);
    unpublished.prepare_user_frame().unwrap();
    assert_eq!(kernel_buffers::test_used(principal), 32);
    drop(unpublished);
    assert_eq!(kernel_buffers::test_used(principal), 0);
    assert_eq!(pool(), before);
    let first = domain(principal);
    let second = domain(principal);
    let healthy = domain(other);
    kernel_buffers::test_limit(principal, 32);
    {
        let mut table = memory::ADDRESS_SPACE_TABLE.lock();
        let space = table.get_mut(first.id()).unwrap();
        for n in 0..4 {
            install(space, n);
        }
        assert_eq!(space.owned_frames.capacity_bytes(), 32);
    }
    let full = pool();
    let mut diagnostic = crate::klib::collections::snapshot::Budget::with_allocator(
        1024,
        kernel_buffers::telemetry_allocator(first.id()).unwrap(),
    );
    assert!(
        diagnostic.prepare::<u8>(1).is_err(),
        "diagnostics bypassed the same principal ceiling"
    );
    let frames = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for _ in 0..128 {
        let mut table = memory::ADDRESS_SPACE_TABLE.lock();
        for handle in [first, second] {
            assert!(matches!(
                PreparingUserBacking::new(table.get_mut(handle.id()).unwrap(), Kind::Heap),
                Err(memory::BackingPreparationError::Tracking)
            ));
        }
    }
    assert_eq!(pool(), full);
    assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
    memory::budget::mark_platform(first);
    assert_eq!(pool(), full);
    kernel_buffers::test_limit(principal, 96); // 32 old + 64 candidate.
    {
        let mut table = memory::ADDRESS_SPACE_TABLE.lock();
        install(table.get_mut(first.id()).unwrap(), 4);
        install(table.get_mut(healthy.id()).unwrap(), 0);
    }
    assert_eq!(kernel_buffers::test_used(principal), 64);
    assert_eq!(kernel_buffers::test_used(other), 32);
    assert_eq!(pool().1[0], full.1[0] + 32 + 32, "growth reclassified original ordinary backing");
    memory::close_user_address_space_handle(first).unwrap();
    assert_eq!(kernel_buffers::test_used(principal), 0);
    let replacement = domain(other);
    assert_eq!(replacement.id(), first.id());
    assert_ne!(replacement, first);
    assert_eq!(kernel_buffers::test_used(other), 32);
    for handle in [second, healthy, replacement] {
        memory::close_user_address_space_handle(handle).unwrap();
    }
    assert_eq!(pool(), before);

    // Deny only catalog admission before the first data frame, leaving all
    // real physical counters and the principal's unused reservation unchanged.
    let mut denied = root(principal);
    let limit = crate::resource_policy::POLICY.memory_capability_nodes.ordinary[0];
    let pressure =
        Reservation::<MemoryCapabilityNodes, 2>::reserve(false, [limit - pool().1[0], 0]).unwrap();
    let free = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert!(matches!(
        PreparingUserBacking::new(&mut denied, Kind::Heap),
        Err(memory::BackingPreparationError::Tracking)
    ));
    assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(kernel_buffers::test_used(principal), 0);
    drop(pressure);
    drop(denied);
    late_binding();
    inert_abandonment(principal);
    crate::logln!(
        "[root frame tracking] signed unpublished capacity, shared diagnostic/root principal \
         growth, 256 pre-frame denials, original ordinary class, healthy root, close/reuse, \
         late-binding atomicity and guarded retention passed"
    );
}

fn late_binding() {
    let principal = charlotte_launch::artifact_principal_id(b"root-frame-buffer-late");
    drop(memory::prepare_principal_backing(principal, false).unwrap());
    let mut space = AddressSpace::try_new_user().unwrap();
    install(&mut space, 0);
    let handle = memory::register_user_address_space(space).unwrap();
    let before = pool();
    kernel_buffers::test_limit(principal, 31);
    assert_eq!(
        memory::register_domain_authority(handle, principal, 0),
        Err(memory::DomainAuthorityError::ResourceLimit)
    );
    assert_eq!(kernel_buffers::test_used(principal), 0);
    assert!(memory::domain_authority(handle.id()).is_none());
    kernel_buffers::test_limit(principal, 32);
    kernel_buffers::test_with_backing_limit(principal, 0, || {
        assert_eq!(
            memory::register_domain_authority(handle, principal, 0),
            Err(memory::DomainAuthorityError::ResourceLimit)
        );
        assert_eq!(kernel_buffers::test_used(principal), 0);
        assert!(memory::domain_authority(handle.id()).is_none());
    });
    assert_eq!(pool(), before);
    memory::register_domain_authority(handle, principal, 0).unwrap();
    assert_eq!(kernel_buffers::test_used(principal), 32);
    assert_eq!(pool(), before);
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(kernel_buffers::test_used(principal), 0);
}

fn inert_abandonment(principal: u64) {
    let root = root(principal);
    let mut tracker = FrameTracker::new(&root.table_account);
    tracker.prepare().unwrap();
    let charged = pool();
    memory::usage::with_principal_history_locked(|| {
        let _heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        admission::with_locked::<MemoryCapabilityNodes, 2, _>(|| drop(tracker));
    });
    assert_eq!(pool(), charged);
    assert_eq!(kernel_buffers::test_used(principal), 32);
    drop(root);
    assert_eq!(kernel_buffers::test_used(principal), 32);
}
