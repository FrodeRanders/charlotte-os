//! Actual shared backing exhaustion, including all root preparation phases.
use super::*;
use crate::{
    klib::namespace_account_budget as backing,
    memory::{
        self,
        AddressSpace,
        AddressSpaceRegistrationError,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    },
};

fn reject_root(expected: AddressSpaceRegistrationError) {
    let before = backing::snapshot();
    let frames = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let roots = memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count();
    assert_eq!(
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()),
        Err(expected)
    );
    assert_eq!(backing::snapshot(), before);
    assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
    assert_eq!(
        memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count(),
        roots
    );
}

fn denied(platform: bool) {
    assert!(budget::DomainBudget::try_new(platform).is_err());
    crate::ipc::namespace::assert_account_backing_denied(platform);
    crate::syscall::mailbox_tests::assert_account_backing_denied(platform);
}

pub(super) fn run() {
    let before = backing::snapshot();
    let root = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    crate::syscall::mailbox_tests::prepare_account_backing_fixture(root);
    let mut ordinary = alloc::vec::Vec::with_capacity(backing::ORDINARY_LIMIT as usize);
    while let Ok(account) = budget::DomainBudget::try_new(false) {
        assert_eq!(account.used(), 0);
        ordinary.push(account);
        assert!(ordinary.len() <= backing::ORDINARY_LIMIT as usize);
    }
    assert_eq!(backing::snapshot().1, backing::ORDINARY_LIMIT);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            denied(false);
        }
    }
    for available in 0..3 {
        for _ in 0..available {
            drop(ordinary.pop().unwrap());
        }
        reject_root(
            if available == 0 {
                AddressSpaceRegistrationError::CapabilityNamespaceAllocationFailed
            } else {
                AddressSpaceRegistrationError::IpcNamespaceAllocationFailed
            },
        );
        for _ in 0..available {
            ordinary.push(budget::DomainBudget::try_new(false).unwrap());
        }
    }
    crate::syscall::mailbox_tests::assert_account_pressure_preserves_existing(root);
    let full_ordinary = backing::snapshot();
    memory::budget::mark_platform(root);
    assert_eq!(backing::snapshot(), full_ordinary);
    let trusted =
        memory::register_user_address_space(AddressSpace::try_new_platform_user().unwrap())
            .unwrap();
    crate::ipc::namespace::assert_ordinary_backing_fixture(trusted);
    let resource = budget::node_used();
    let cap = try_allocate(trusted.id(), ObjectKind::SystemObserver).unwrap();
    assert_eq!(budget::node_used(), (resource.0 + 1, resource.1 + 1));
    assert!(remove(trusted.id(), cap, ObjectKind::SystemObserver));
    crate::syscall::mailbox_tests::prepare_account_backing_fixture(trusted);
    assert_eq!(backing::snapshot(), (full_ordinary.0 + 4, full_ordinary.1));
    memory::close_user_address_space_handle(trusted).unwrap();
    assert_eq!(backing::snapshot(), full_ordinary);
    let mut reserved =
        alloc::vec::Vec::with_capacity((backing::NODE_LIMIT - backing::ORDINARY_LIMIT) as usize);
    while let Ok(account) = budget::DomainBudget::try_new(true) {
        reserved.push(account);
        assert!(reserved.len() <= (backing::NODE_LIMIT - backing::ORDINARY_LIMIT) as usize);
    }
    let full = backing::snapshot();
    assert_eq!(full.0, backing::NODE_LIMIT);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        denied(true);
    }
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(backing::snapshot(), (full.0 - 4, full.1 - 4));
    let recovered =
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    crate::syscall::mailbox_tests::prepare_account_backing_fixture(recovered);
    assert_eq!(backing::snapshot(), full);
    memory::close_user_address_space_handle(recovered).unwrap();
    let counts = (ordinary.len(), reserved.len());
    drop((ordinary, reserved));
    assert_eq!(backing::snapshot(), before);
    crate::logln!(
        "[namespace account pressure] {} ordinary + {} reserved actual accounts; {}/{} ceilings, \
         256 heap-held four-constructor denials, zero/one/two-slot root rollback, trusted backing \
         without authority/IPC/mailbox privilege, existing receiver/word progress and immutable \
         promotion/close/recovery passed",
        counts.0,
        counts.1,
        backing::ORDINARY_LIMIT,
        backing::NODE_LIMIT
    );
    principal_backing();
}

fn signed_root(name: &[u8]) -> (memory::AddressSpaceHandle, u64) {
    let principal = charlotte_launch::artifact_principal_id(name);
    let space = AddressSpace::try_new_principal_user(
        false,
        memory::prepare_principal_backing(principal, false).unwrap(),
    )
    .unwrap();
    let root = memory::register_user_address_space(space).unwrap();
    memory::register_domain_authority(root, principal, 0).unwrap();
    (root, principal)
}
fn storage(root: memory::AddressSpaceHandle) -> memory::kernel_buffers::CapturedStorage {
    memory::kernel_buffers::capture_storage(root).unwrap()
}
fn principal_backing() {
    use memory::kernel_buffers::{
        test_account_limit as limit,
        test_account_used as used,
    };
    let before = backing::snapshot();
    let (first, principal) = signed_root(b"namespace-account-principal");
    let mandatory = used(principal);
    assert!(mandatory > 0);
    let (sibling, same) = signed_root(b"namespace-account-principal");
    assert_eq!(same, principal);
    assert_eq!(used(principal), 2 * mandatory);
    let (healthy, other) = signed_root(b"namespace-account-healthy");
    assert_eq!(used(other), mandatory);
    let policy = storage(first);
    let sample = budget::DomainBudget::try_new(policy).unwrap();
    let cap_bytes = used(principal) - 2 * mandatory;
    drop(sample);
    let [endpoint_bytes, record_bytes] =
        crate::ipc::namespace::account_bytes_for_test(policy, principal);
    let mailbox_bytes = crate::syscall::mailbox_tests::account_bytes_for_test(policy, principal);
    assert_eq!(mandatory, cap_bytes + endpoint_bytes + record_bytes);
    crate::syscall::mailbox_tests::prepare_account_backing_fixture(first);
    let shared = 2 * mandatory + mailbox_bytes;
    assert_eq!(used(principal), shared);
    let cap_weak =
        alloc::sync::Arc::downgrade(&CAPABILITIES.lock().get(&first.id()).unwrap().budget);
    let (endpoint_weak, record_weak) = crate::ipc::namespace::account_weaks_for_test(first.id());
    let mailbox_weak = crate::syscall::mailbox_tests::account_weak_for_test(first.id());
    let family = backing::snapshot();
    memory::budget::mark_platform(first);
    assert_eq!(backing::snapshot(), family);
    limit(principal, shared);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            let policy = storage(first);
            assert!(budget::DomainBudget::try_new(policy).is_err());
            crate::ipc::namespace::assert_account_backing_denied(policy);
            crate::syscall::mailbox_tests::assert_account_backing_denied(policy);
            assert_eq!(used(principal), shared);
            assert_eq!(backing::snapshot(), family);
        }
    }
    let independent = budget::DomainBudget::try_new(storage(healthy)).unwrap();
    assert_eq!(used(other), mandatory + cap_bytes);
    drop(independent);
    // Reject each actual root prefix: no account, cap only, then cap+endpoint.
    for headroom in [0, cap_bytes, cap_bytes + endpoint_bytes] {
        limit(principal, shared + headroom);
        let frames = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let roots = memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count();
        let root = AddressSpace::try_new_principal_user(
            false,
            memory::prepare_principal_backing(principal, false).unwrap(),
        )
        .unwrap();
        assert_eq!(
            memory::register_user_address_space(root),
            Err(if headroom == 0 {
                AddressSpaceRegistrationError::CapabilityNamespaceAllocationFailed
            } else {
                AddressSpaceRegistrationError::IpcNamespaceAllocationFailed
            })
        );
        assert_eq!(used(principal), shared);
        assert_eq!(backing::snapshot(), family);
        assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
        assert_eq!(
            memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count(),
            roots
        );
    }
    limit(principal, crate::resource_policy::POLICY.memory.principal_namespace_account_bytes);
    memory::close_user_address_space_handle(first).unwrap();
    assert!(cap_weak.upgrade().is_none() && endpoint_weak() && record_weak() && mailbox_weak());
    assert_eq!(used(principal), shared, "weak-only backing was refunded at root exit");
    let (replacement, replacement_principal) = signed_root(b"namespace-account-healthy");
    assert_eq!(replacement.id(), first.id());
    assert_ne!(replacement, first);
    assert_eq!(replacement_principal, other);
    let healthy_bytes = used(other);
    let family = backing::snapshot();
    drop(cap_weak);
    assert_eq!(used(principal), shared - cap_bytes);
    assert_eq!(backing::snapshot(), (family.0 - 1, family.1 - 1));
    drop(endpoint_weak);
    assert_eq!(used(principal), shared - cap_bytes - endpoint_bytes);
    drop(record_weak);
    assert_eq!(used(principal), mandatory + mailbox_bytes);
    drop(mailbox_weak);
    assert_eq!(used(principal), mandatory);
    assert_eq!(used(other), healthy_bytes);
    for root in [sibling, healthy, replacement] {
        memory::close_user_address_space_handle(root).unwrap();
    }
    assert_eq!(used(principal), 0);
    assert_eq!(used(other), 0);
    assert_eq!(backing::snapshot(), before);
    // Allocate real complete capability-account bundles until the unchanged
    // node pool still has room but this principal's actual-byte limit binds.
    let (root, pressure_principal) = signed_root(b"namespace-account-byte-pressure");
    let policy = storage(root);
    let baseline = used(pressure_principal);
    let ceiling = crate::resource_policy::POLICY.memory.principal_namespace_account_bytes;
    let mut accounts = alloc::vec::Vec::with_capacity((ceiling / cap_bytes) as usize + 1);
    while let Ok(account) = budget::DomainBudget::try_new(policy) {
        assert!(accounts.len() < (ceiling / cap_bytes) as usize + 1);
        accounts.push(account);
    }
    let full = used(pressure_principal);
    assert!(ceiling - full < cap_bytes);
    assert!(backing::snapshot().1 < backing::ORDINARY_LIMIT);
    let count = accounts.len();
    drop(accounts);
    assert_eq!(used(pressure_principal), baseline);
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(used(pressure_principal), 0);
    assert_eq!(backing::snapshot(), before);
    crate::logln!(
        "[namespace account principal] actual cap/endpoint/record/mailbox bundles={}/{}/{}/{} \
         bytes; sibling sharing, 256 heap-held denials, independent sponsor, three root-prefix \
         rollbacks, promotion, four final-weak refunds after ASID reuse and {} real bundles at \
         {}-byte principal ceiling passed",
        cap_bytes,
        endpoint_bytes,
        record_bytes,
        mailbox_bytes,
        count,
        ceiling
    );
}
