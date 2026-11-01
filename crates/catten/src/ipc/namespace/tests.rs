//! Serialized controls for namespace publication and allocator-free fallback.
use super::*;
use crate::memory::{
    ADDRESS_SPACE_LIFECYCLE,
    ADDRESS_SPACE_TABLE,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

pub(super) fn run() {
    node_budget::test_allocation::<(AddressSpaceId, AsIpcCaps)>("ipc");
    let missing = fixture_id(0x0e_b120);
    assert!(IPC.read().caps.get(&missing).is_none());
    assert_eq!(endpoint_create(missing, 1, 1, 1), Err(IpcError::PermissionDenied));
    assert!(IPC.read().caps.get(&missing).is_none());

    let root = crate::service::loader::create_user_address_space_handle();
    let operation = AddressSpaceOperation::acquire(root).unwrap();
    let mut local = IpcRegistry::new();
    for _ in 0..64 {
        let nodes = node_budget::snapshot();
        let backing = crate::klib::namespace_account_budget::snapshot();
        let prepared = PreparingNamespace::try_new().unwrap();
        let endpoints = Arc::downgrade(prepared.0.endpoints.as_ref().unwrap());
        let records = Arc::downgrade(prepared.0.records.as_ref().unwrap());
        let retired = {
            let _heap = PRIMARY_ALLOCATOR.lock();
            crate::klib::namespace_account_budget::with_locked(|| {
                node_budget::with_locked(|| {
                    prepared.publish_into(&mut local, root.id(), Some(operation.handle()));
                });
            });
            assert_eq!(local.caps[&root.id()].address_space, Some(root));
            let retired = local.caps.take(&root.id()).unwrap();
            assert_eq!(endpoints.strong_count(), 1);
            assert_eq!(records.strong_count(), 1);
            retired
        };
        // An empty node may be disposed outside IPC; independently retained
        // charge references are exercised by the real delegation/reuse tests.
        retired.release();
        assert_eq!(node_budget::snapshot(), nodes);
        assert_eq!(endpoints.strong_count(), 0);
        assert_eq!(records.strong_count(), 0);
        assert_eq!(
            crate::klib::namespace_account_budget::snapshot(),
            (backing.0 + 2, backing.1 + 2)
        );
        drop((endpoints, records));
        assert_eq!(crate::klib::namespace_account_budget::snapshot(), backing);
    }
    operation.release().unwrap();
    crate::memory::close_user_address_space_handle(root).unwrap();

    let retained = crate::klib::namespace_account_budget::snapshot();
    let retained_nodes = node_budget::snapshot();
    for phase in 1..=3 {
        let mut partial = PreparingNamespace(ManuallyDrop::new(Resources {
            node: Some(node_budget::prepare(false).unwrap()),
            endpoints: None,
            records: None,
        }));
        if phase >= 2 {
            partial.0.endpoints = Some(budget::DomainBudget::try_new(false).unwrap());
        }
        if phase == 3 {
            partial.0.records = Some(record_budget::DomainBudget::try_new(false).unwrap());
        }
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let _table = ADDRESS_SPACE_TABLE.lock();
        let _ipc = IPC.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        crate::klib::namespace_account_budget::with_locked(|| {
            node_budget::with_locked(|| drop(partial));
        });
    }

    // A detached complete node must retain both accounts in an unknown Drop
    // context too. No callback, account release or allocator entry is allowed.
    let prepared = PreparingNamespace::try_new().unwrap();
    let endpoints = Arc::downgrade(prepared.0.endpoints.as_ref().unwrap());
    let records = Arc::downgrade(prepared.0.records.as_ref().unwrap());
    prepared.publish_into(&mut local, missing, None);
    let retired = local.caps.take(&missing).unwrap();
    {
        let _ipc = IPC.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        crate::klib::namespace_account_budget::with_locked(|| {
            node_budget::with_locked(|| drop(retired));
        });
    }
    assert_eq!(endpoints.strong_count(), 1);
    assert_eq!(records.strong_count(), 1);
    assert_eq!(crate::klib::namespace_account_budget::snapshot(), (retained.0 + 5, retained.1 + 5));
    let bytes = node_budget::Prepared::<(AddressSpaceId, AsIpcCaps)>::allocation_bytes() as u64;
    assert_eq!(
        node_budget::snapshot(),
        (
            [retained_nodes.0[0] + 4 * bytes, retained_nodes.0[1] + 4],
            [retained_nodes.1[0] + 4 * bytes, retained_nodes.1[1] + 4]
        )
    );
    crate::logln!(
        "[IPC namespace preparation] missing namespace rejects, 64 heap-held \
         publications/detaches, account disposal and guarded partial/retired retention passed; \
         four standalone nodes and five budget accounts deliberately retained"
    );
}
