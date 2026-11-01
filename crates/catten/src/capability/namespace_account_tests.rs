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
}
