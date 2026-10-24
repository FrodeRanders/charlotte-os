//! Root transaction controls around the IPC publication boundary.
use super::*;

#[allow(clippy::result_large_err)]
pub(super) fn run() {
    crate::ipc::namespace::test_preparation();
    for reject in 1..=4 {
        for _ in 0..64 {
            let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
            let roots = ADDRESS_SPACE_TABLE.lock().iter().filter(|r| r.is_some()).count();
            let metadata = metadata::admission::snapshot();
            let capabilities = crate::capability::node_admission_used();
            assert_eq!(
                register_with_ipc(
                    AddressSpace::try_new_user().unwrap(),
                    |_, _| panic!("rejected IPC preparation published a root"),
                    |root| {
                        assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                        assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
                        assert!(budget::tests::ledger_available());
                        drop(root);
                    },
                    budget::PreparingAccount::try_new,
                    || metadata::PreparingMetadata::try_new(false),
                    || crate::ipc::namespace::PreparingNamespace::test_reject_at(reject),
                    |_| panic!("rejected IPC preparation reached publication"),
                ),
                Err(AddressSpaceRegistrationError::IpcNamespaceAllocationFailed),
            );
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
            assert_eq!(ADDRESS_SPACE_TABLE.lock().iter().filter(|r| r.is_some()).count(), roots);
            assert_eq!(metadata::admission::snapshot(), metadata);
            assert_eq!(crate::capability::node_admission_used(), capabilities);
        }
    }
    for _ in 0..64 {
        let root = register_with_ipc(
            AddressSpace::try_new_user().unwrap(),
            IdTable::try_add_element,
            drop,
            budget::PreparingAccount::try_new,
            || metadata::PreparingMetadata::try_new(false),
            crate::ipc::namespace::PreparingNamespace::try_new,
            |operation| {
                let handle = operation.handle();
                assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
                assert_eq!(
                    crate::ipc::endpoint_create(handle.id(), 1, 1, 1),
                    Err(crate::ipc::IpcError::PermissionDenied)
                );
                assert_eq!(
                    close_user_address_space_handle(handle),
                    Err(AddressSpaceCloseError::OperationsInFlight)
                );
            },
        )
        .unwrap();
        let endpoint = crate::ipc::endpoint_create(root.id(), 1, 1, 1).unwrap();
        crate::ipc::close_cap(root.id(), endpoint).unwrap();
        close_user_address_space_handle(root).unwrap();
        assert_eq!(
            crate::ipc::endpoint_create(root.id(), 1, 1, 1),
            Err(crate::ipc::IpcError::PermissionDenied)
        );
    }
    crate::logln!(
        "[IPC root publication] 256 phase rejections release the complete unpublished root; 64 \
         publication leases prevent early close and missing/retired namespaces never recreate"
    );
}
