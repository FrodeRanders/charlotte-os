//! Actual candidate-node pressure, distinct from published capability counts.
use super::*;
use crate::memory::{
    self,
    AddressSpace,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn root(platform: bool) -> memory::AddressSpaceHandle {
    let space = if platform {
        AddressSpace::try_new_platform_user()
    } else {
        AddressSpace::try_new_user()
    };
    memory::register_user_address_space(space.unwrap()).unwrap()
}
fn denied(root: memory::AddressSpaceHandle) {
    assert!(matches!(
        PreparedReservation::try_new(root.id(), ObjectKind::Memory, Some(root)),
        Err(AllocationError::ResourceLimit)
    ));
}
pub(super) fn run() {
    record_budget::test_rejection();
    let before = record_budget::snapshot();
    let capabilities = budget::node_used();
    let retained_principal =
        charlotte_launch::artifact_principal_id(b"authority-node-retained-principal");
    let old = funded_root(retained_principal);
    let root_bytes = memory::kernel_buffers::test_metadata_used(retained_principal);
    let trusted = root(true);
    let old_account = CAPABILITIES.lock().get(&old.id()).unwrap().budget.clone();
    let old_cap = try_allocate(old.id(), ObjectKind::Memory).unwrap();
    let retained =
        retire_record(old.id(), old_cap, ObjectKind::Memory, |s| s == EntryState::Live).unwrap();
    let bytes = record_budget::Prepared::allocation_bytes() as u64;
    assert_eq!(memory::kernel_buffers::test_metadata_used(retained_principal), root_bytes + bytes);
    assert_eq!(
        record_budget::snapshot(),
        ([before.0[0] + bytes, before.0[1] + 1], [before.1[0] + bytes, before.1[1] + 1])
    );

    let mut ordinary = alloc::vec::Vec::with_capacity(record_budget::ORDINARY[1] as usize);
    while let Ok(node) = record_budget::prepare(false) {
        ordinary.push(node);
        assert!(ordinary.len() <= record_budget::ORDINARY[1] as usize);
    }
    let full_ordinary = record_budget::snapshot();
    assert_eq!(full_ordinary.1[1], record_budget::ORDINARY[1]);
    // Admission rejects before touching the heap, even after resource promotion.
    mark_platform(old);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            denied(old);
        }
    }
    assert_eq!(record_budget::snapshot(), full_ordinary);
    assert_eq!(CAPABILITIES.lock().get(&old.id()).unwrap().next_serial, old_cap + 1);
    assert_eq!(budget::node_used(), (capabilities.0 + 1, capabilities.1 + 1));

    // Trusted backing alone must not grant platform capability admission.
    let trusted_cap = try_allocate(trusted.id(), ObjectKind::Memory).unwrap();
    assert_eq!(
        record_budget::snapshot(),
        ([full_ordinary.0[0] + bytes, full_ordinary.0[1] + 1], full_ordinary.1)
    );
    assert_eq!(budget::node_used(), (capabilities.0 + 2, capabilities.1 + 2));
    let mut reserve = alloc::vec::Vec::with_capacity(
        (record_budget::LIMIT[1] - record_budget::ORDINARY[1]) as usize,
    );
    while let Ok(node) = record_budget::prepare(true) {
        reserve.push(node);
        assert!(reserve.len() <= (record_budget::LIMIT[1] - record_budget::ORDINARY[1]) as usize);
    }
    let full = record_budget::snapshot();
    assert_eq!(full.0[1], record_budget::LIMIT[1]);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            denied(trusted);
        }
    }
    memory::close_user_address_space_handle(old).unwrap();
    let successor = root(false);
    assert_eq!(successor.id(), old.id());
    assert_ne!(successor, old);
    assert_eq!(old_account.used(), 1);
    assert_eq!(record_budget::snapshot(), full);
    // Exact-generation preflight cannot allocate against the successor.
    record_budget::with_locked(|| {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(matches!(
            PreparedReservation::try_new(old.id(), ObjectKind::Memory, Some(old)),
            Err(AllocationError::Retired)
        ));
    });
    assert_eq!(memory::kernel_buffers::test_metadata_used(retained_principal), bytes);
    retained.release();
    assert_eq!(memory::kernel_buffers::test_metadata_used(retained_principal), 0);
    assert_eq!(old_account.used(), 0);
    assert_eq!(
        record_budget::snapshot(),
        ([full.0[0] - bytes, full.0[1] - 1], [full.1[0] - bytes, full.1[1] - 1])
    );
    let recovered = try_allocate(successor.id(), ObjectKind::Memory).unwrap();
    assert_eq!(record_budget::snapshot(), full);
    assert!(remove(successor.id(), recovered, ObjectKind::Memory));
    assert!(remove(trusted.id(), trusted_cap, ObjectKind::Memory));
    memory::close_user_address_space_handle(successor).unwrap();
    memory::close_user_address_space_handle(trusted).unwrap();
    let counts = (ordinary.len(), reserve.len());
    drop((ordinary, reserve));
    assert_eq!(record_budget::snapshot(), before);
    assert_eq!(budget::node_used(), capabilities);
    crate::logln!(
        "[authority node backing] bytes={} actual candidates ordinary={} trusted={}; \
         ordinary/total ceilings {}/{}; 64 allocation failures, byte denials, 128 heap-held \
         admission denials, immutable promotion, trusted storage without grants, retained record \
         across root close/ASID reuse, exact recovery passed",
        bytes,
        counts.0,
        counts.1,
        record_budget::ORDINARY[1],
        record_budget::LIMIT[1]
    );
    principal_funding();
}

fn funded_root(principal: u64) -> memory::AddressSpaceHandle {
    let space = AddressSpace::try_new_principal_user(
        false,
        memory::prepare_principal_backing(principal, false).unwrap(),
    )
    .unwrap();
    memory::register_user_address_space(space).unwrap()
}
fn principal_funding() {
    use memory::kernel_buffers::{
        test_metadata_limit as limit,
        test_metadata_used as used,
    };
    let before = record_budget::snapshot();
    let principal = charlotte_launch::artifact_principal_id(b"authority-mailbox-node-principal");
    let other = charlotte_launch::artifact_principal_id(b"authority-mailbox-node-healthy");
    let first = funded_root(principal);
    let root_bytes = used(principal);
    assert!(root_bytes > 0);
    let second = funded_root(principal);
    let healthy = funded_root(other);
    let storage = CAPABILITIES.lock().get(&first.id()).unwrap().storage;
    assert_eq!(used(principal), 2 * root_bytes);
    record_budget::test_principal_rejection(storage);
    assert_eq!(used(principal), 2 * root_bytes);
    let a = try_allocate(first.id(), ObjectKind::SystemObserver).unwrap();
    let c = try_allocate(second.id(), ObjectKind::SystemObserver).unwrap();
    let bytes = record_budget::Prepared::allocation_bytes() as u64;
    assert_eq!(used(principal), 2 * root_bytes + 2 * bytes);
    let payload = crate::syscall::mailbox_tests::test_metadata_principal(
        first, healthy, principal, other, bytes,
    );
    let shared = used(principal);
    let healthy_base = used(other);
    limit(principal, shared);
    let nodes = record_budget::snapshot();
    mark_platform(first);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..128 {
            denied(first);
        }
    }
    assert_eq!(used(principal), shared);
    assert_eq!(record_budget::snapshot(), nodes);
    let independent = try_allocate(healthy.id(), ObjectKind::SystemObserver).unwrap();
    assert_eq!(used(other), healthy_base + bytes);
    assert!(remove(healthy.id(), independent, ObjectKind::SystemObserver));
    let retained =
        retire_record(first.id(), a, ObjectKind::SystemObserver, |s| s == EntryState::Live)
            .unwrap();
    memory::close_user_address_space_handle(first).unwrap();
    assert_eq!(used(principal), root_bytes + 2 * bytes);
    let replacement = funded_root(other);
    assert_eq!(replacement.id(), first.id());
    assert_ne!(replacement, first);
    let new = try_allocate(replacement.id(), ObjectKind::SystemObserver).unwrap();
    assert_eq!(used(other), healthy_base + root_bytes + bytes);
    retained.release();
    assert_eq!(used(principal), root_bytes + bytes);
    assert_eq!(used(other), healthy_base + root_bytes + bytes);
    assert!(remove(second.id(), c, ObjectKind::SystemObserver));
    assert_eq!(used(principal), root_bytes);
    assert!(remove(replacement.id(), new, ObjectKind::SystemObserver));
    limit(principal, crate::resource_policy::POLICY.memory.principal_metadata_bytes);
    for root in [second, healthy, replacement] {
        memory::close_user_address_space_handle(root).unwrap();
    }
    assert_eq!(used(other), 0);
    assert_eq!(record_budget::snapshot(), before);
    let pressure = funded_root(principal);
    let storage = CAPABILITIES.lock().get(&pressure.id()).unwrap().storage;
    let pressure_base = used(principal);
    let ceiling = crate::resource_policy::POLICY.memory.principal_metadata_bytes;
    let mut held = alloc::vec::Vec::with_capacity((ceiling / bytes) as usize + 1);
    while let Ok(node) = record_budget::prepare(storage) {
        held.push(node);
        assert!(held.len() <= (ceiling / bytes) as usize);
    }
    assert!(ceiling - used(principal) < bytes);
    assert!(record_budget::snapshot().1[1] < record_budget::ORDINARY[1]);
    let count = held.len();
    drop(held);
    assert_eq!(used(principal), pressure_base);
    memory::close_user_address_space_handle(pressure).unwrap();
    assert_eq!(used(principal), 0);
    assert_eq!(record_budget::snapshot(), before);
    crate::logln!(
        "[record node principal] authority/mailbox payload layouts={}/{} bytes; sibling sharing, \
         128 funded allocator failures, principal denials before allocation, six mailbox \
         prefixes, healthy sponsor, promotion, retained original record across exact ASID reuse \
         and {} real candidates at {}-byte ceiling passed",
        bytes,
        payload,
        count,
        ceiling
    );
}
