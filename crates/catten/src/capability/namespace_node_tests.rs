//! Serialized real node pressure; account and payload allocations are separate.
use super::*;
use crate::memory::{
    self,
    AddressSpace,
    AddressSpaceRegistrationError,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn rejected_root(expected: AddressSpaceRegistrationError) {
    let nodes = node_budget::snapshot();
    let accounts = crate::klib::namespace_account_budget::snapshot();
    let frames = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let roots = memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count();
    assert_eq!(
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()),
        Err(expected)
    );
    assert_eq!(node_budget::snapshot(), nodes);
    assert_eq!(crate::klib::namespace_account_budget::snapshot(), accounts);
    assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
    assert_eq!(
        memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count(),
        roots
    );
}

fn complete_root(platform: bool) -> memory::AddressSpaceHandle {
    let space = if platform {
        AddressSpace::try_new_platform_user().unwrap()
    } else {
        AddressSpace::try_new_user().unwrap()
    };
    let root = memory::register_user_address_space(space).unwrap();
    crate::completion::open_address_space_with_cq(root.id(), 4, 2).unwrap();
    crate::syscall::mailbox_tests::prepare_account_backing_fixture(root);
    root
}

pub(super) fn run() {
    node_budget::test_byte_denial();
    let before = node_budget::snapshot();
    let root = complete_root(false);
    let live = node_budget::snapshot();
    let root_bytes = live.0[0] - before.0[0];
    assert_eq!(live.0[1], before.0[1] + 5);
    assert_eq!(live.1, [before.1[0] + root_bytes, before.1[1] + 5]);
    let mut ordinary = alloc::vec::Vec::with_capacity(node_budget::ORDINARY_LIMIT[1] as usize);
    while let Ok(node) = node_budget::prepare::<(AddressSpaceId, AddressSpaceCapabilities)>(false) {
        ordinary.push(node);
        assert!(ordinary.len() <= node_budget::ORDINARY_LIMIT[1] as usize);
    }
    assert_eq!(node_budget::snapshot().1[1], node_budget::ORDINARY_LIMIT[1]);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            assert!(
                node_budget::prepare::<(AddressSpaceId, AddressSpaceCapabilities)>(false).is_err()
            );
        }
    }
    rejected_root(AddressSpaceRegistrationError::CapabilityNamespaceAllocationFailed);
    drop(ordinary.pop().unwrap());
    rejected_root(AddressSpaceRegistrationError::IpcNamespaceAllocationFailed);
    ordinary.push(node_budget::prepare(false).unwrap());
    let full_ordinary = node_budget::snapshot();
    assert!(matches!(
        crate::completion::open_address_space_with_cq(root.id(), 4, 2),
        Err(crate::completion::CqOpenError::AllocationFailed)
    ));
    assert_eq!(node_budget::snapshot(), full_ordinary);
    crate::syscall::mailbox_tests::assert_account_pressure_preserves_existing(root);
    memory::budget::mark_platform(root);
    assert_eq!(node_budget::snapshot(), full_ordinary);
    let trusted = complete_root(true);
    crate::ipc::namespace::assert_ordinary_backing_fixture(trusted);
    let admitted = node_budget::snapshot();
    assert_eq!(admitted.1, full_ordinary.1);
    assert_eq!(admitted.0, [full_ordinary.0[0] + root_bytes, full_ordinary.0[1] + 5]);
    memory::close_user_address_space_handle(trusted).unwrap();
    assert_eq!(node_budget::snapshot(), full_ordinary);
    let mut reserve = alloc::vec::Vec::with_capacity(
        (node_budget::NODE_LIMIT[1] - node_budget::ORDINARY_LIMIT[1]) as usize,
    );
    while let Ok(node) = node_budget::prepare::<(AddressSpaceId, AddressSpaceCapabilities)>(true) {
        reserve.push(node);
        assert!(
            reserve.len() <= (node_budget::NODE_LIMIT[1] - node_budget::ORDINARY_LIMIT[1]) as usize
        );
    }
    let full = node_budget::snapshot();
    assert_eq!(full.0[1], node_budget::NODE_LIMIT[1]);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(node_budget::prepare::<(AddressSpaceId, AddressSpaceCapabilities)>(true).is_err());
    }
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(
        node_budget::snapshot(),
        ([full.0[0] - root_bytes, full.0[1] - 5], [full.1[0] - root_bytes, full.1[1] - 5])
    );
    let recovered = complete_root(false);
    assert_eq!(node_budget::snapshot(), full);
    memory::close_user_address_space_handle(recovered).unwrap();
    let counts = (ordinary.len(), reserve.len());
    drop((ordinary, reserve));
    assert_eq!(node_budget::snapshot(), before);
    crate::logln!(
        "[namespace node pressure] {} ordinary + {} reserved actual capability-layout nodes; \
         {}/{} node ceilings, byte denial, heap-held denial, root rollback, completion/mailbox \
         rejection, trusted five-node reserve, immutable promotion and close/recovery passed",
        counts.0,
        counts.1,
        node_budget::ORDINARY_LIMIT[1],
        node_budget::NODE_LIMIT[1]
    );
    crate::logln!(
        "[namespace node layouts] complete five-node root {} bytes; count-headroom layouts {} \
         total / {} ordinary bytes",
        root_bytes,
        root_bytes * node_budget::NODE_LIMIT[1] / 5,
        root_bytes * node_budget::ORDINARY_LIMIT[1] / 5
    );
}
