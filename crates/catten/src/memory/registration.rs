//! One owner retains an unpublished root and all prepared publication metadata.
use core::mem::ManuallyDrop;

use super::{
    operation::AddressSpaceOperation,
    *,
};

pub(super) mod metadata;
pub(super) mod table_backing;

struct Resources {
    root: Option<AddressSpace>,
    namespace: Option<crate::capability::PreparingNamespace>,
    account: Option<budget::PreparingAccount>,
    metadata: Option<metadata::PreparingMetadata>,
    ipc: Option<crate::ipc::namespace::PreparingNamespace>,
    publication: Option<AddressSpaceOperation>,
    growth: Option<table_backing::Growth>,
}

#[must_use]
struct PreparingRegistration(ManuallyDrop<Resources>);

impl PreparingRegistration {
    fn capture(root: AddressSpace) -> Self {
        Self(ManuallyDrop::new(Resources {
            root: Some(root),
            namespace: None,
            account: None,
            metadata: None,
            ipc: None,
            publication: None,
            growth: None,
        }))
    }

    fn prepare(
        &mut self,
        account: impl FnOnce() -> Result<budget::PreparingAccount, core::alloc::AllocError>,
        metadata: impl FnOnce() -> Result<metadata::PreparingMetadata, core::alloc::AllocError>,
        ipc: impl FnOnce() -> Result<crate::ipc::namespace::PreparingNamespace, core::alloc::AllocError>,
    ) -> Result<(), AddressSpaceRegistrationError> {
        self.0.namespace = Some(
            crate::capability::prepare_namespace(kernel_buffers::capture_unpublished(
                self.0.root.as_ref().unwrap(),
            ))
            .map_err(|_| AddressSpaceRegistrationError::CapabilityNamespaceAllocationFailed)?,
        );
        self.0.account = Some(
            account().map_err(|_| AddressSpaceRegistrationError::MemorySponsorAllocationFailed)?,
        );
        self.0.metadata = Some(
            metadata()
                .map_err(|_| AddressSpaceRegistrationError::DomainMetadataAllocationFailed)?,
        );
        self.0.ipc =
            Some(ipc().map_err(|_| AddressSpaceRegistrationError::IpcNamespaceAllocationFailed)?);
        self.prepare_growth()
    }

    fn prepare_growth(&mut self) -> Result<(), AddressSpaceRegistrationError> {
        self.0.growth = table_backing::prepare(self.0.root.as_ref().unwrap())
            .map_err(|_| AddressSpaceRegistrationError::TableAllocationFailed)?;
        Ok(())
    }

    fn finish_growth(&mut self) {
        if let Some(growth) = self.0.growth.take() {
            growth.finish();
        }
    }

    fn finish(mut self, reject: impl FnOnce(AddressSpace)) {
        self.finish_growth();
        if let Some(ipc) = self.0.ipc.take() {
            ipc.cancel_unpublished();
        }
        if let Some(metadata) = self.0.metadata.take() {
            metadata.cancel_unpublished();
        }
        if let Some(account) = self.0.account.take() {
            account.cancel_unpublished();
        }
        if let Some(namespace) = self.0.namespace.take() {
            namespace.cancel_unpublished();
        }
        if let Some(publication) = self.0.publication.take() {
            publication.release().expect("root publication lost its lease");
        }
        if let Some(root) = self.0.root.take() {
            reject(root);
        }
    }
}

pub(super) fn register(
    root: AddressSpace,
    publish: impl FnMut(
        &mut AddressSpaceTable,
        &mut Option<AddressSpace>,
        Option<&mut table_backing::Growth>,
    ) -> Result<usize, crate::klib::collections::id_table::Error>,
    reject: impl FnOnce(AddressSpace),
    account: impl FnOnce() -> Result<budget::PreparingAccount, core::alloc::AllocError>,
    metadata: impl FnOnce() -> Result<metadata::PreparingMetadata, core::alloc::AllocError>,
) -> Result<AddressSpaceHandle, AddressSpaceRegistrationError> {
    let storage = kernel_buffers::capture_unpublished(&root);
    register_with_ipc(
        root,
        publish,
        reject,
        account,
        metadata,
        || crate::ipc::namespace::PreparingNamespace::try_new_with_backing_class(storage),
        |_| {},
    )
}

fn register_with_ipc(
    root: AddressSpace,
    mut publish: impl FnMut(
        &mut AddressSpaceTable,
        &mut Option<AddressSpace>,
        Option<&mut table_backing::Growth>,
    ) -> Result<usize, crate::klib::collections::id_table::Error>,
    reject: impl FnOnce(AddressSpace),
    account: impl FnOnce() -> Result<budget::PreparingAccount, core::alloc::AllocError>,
    metadata: impl FnOnce() -> Result<metadata::PreparingMetadata, core::alloc::AllocError>,
    ipc: impl FnOnce() -> Result<crate::ipc::namespace::PreparingNamespace, core::alloc::AllocError>,
    before_publication: impl FnOnce(&AddressSpaceOperation),
) -> Result<AddressSpaceHandle, AddressSpaceRegistrationError> {
    let mut owner = PreparingRegistration::capture(root);
    if let Err(error) = owner.prepare(account, metadata, ipc) {
        owner.finish(reject);
        return Err(error);
    }
    let mut published = None;
    // Only unchanged, unstarted capacity conflicts permit re-preparation.
    // Quota/allocation/hardware rejection stops with the complete owner retained.
    for attempt in 0..8 {
        let result = (|| {
            let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
            prepare_user_address_space(owner.0.root.as_mut().unwrap())
                .map_err(|error| (error, false))?;
            let mut table = ADDRESS_SPACE_TABLE.lock();
            let resources = &mut *owner.0;
            let id = publish(&mut table, &mut resources.root, resources.growth.as_mut()).map_err(
                |error| {
                    (
                        AddressSpaceRegistrationError::TableAllocationFailed,
                        error == crate::klib::collections::id_table::Error::CapacityChanged,
                    )
                },
            )?;
            debug_assert_ne!(id, KERNEL_ASID);
            assert!(resources.root.is_none(), "root insertion did not consume its owner");
            let handle = AddressSpaceHandle {
                id,
                generation: table.generation(id).expect("new address space missing generation"),
            };
            resources.account.take().unwrap().publish(handle);
            resources.namespace.take().unwrap().publish(handle);
            resources.metadata.take().unwrap().publish(handle);
            resources.publication =
                Some(AddressSpaceOperation::retain_publication(handle, &mut table));
            Ok(handle)
        })();
        match result {
            Ok(handle) => {
                published = Some(handle);
                break;
            }
            Err((error, retry)) => {
                owner.finish_growth();
                if !retry || attempt == 7 {
                    owner.finish(reject);
                    return Err(error);
                }
                if let Err(error) = owner.prepare_growth() {
                    owner.finish(reject);
                    return Err(error);
                }
            }
        }
    }
    let handle = published.expect("root publication attempts did not terminate");
    before_publication(owner.0.publication.as_ref().unwrap());
    owner.0.ipc.take().unwrap().publish(owner.0.publication.as_ref().unwrap());
    owner.finish(reject);
    Ok(handle)
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn test_abandonment() {
    let accounts = crate::klib::namespace_account_budget::snapshot();
    let principal = charlotte_launch::artifact_principal_id(b"account-registration-retained");
    let root = AddressSpace::try_new_principal_user(
        false,
        prepare_principal_backing(principal, false).unwrap(),
    )
    .unwrap();
    let storage = kernel_buffers::capture_unpublished(&root);
    let mut owner = PreparingRegistration::capture(root);
    owner
        .prepare(
            || budget::PreparingAccount::try_new(storage),
            || metadata::PreparingMetadata::try_new(storage),
            || crate::ipc::namespace::PreparingNamespace::try_new_with_backing_class(storage),
        )
        .unwrap();
    let principal_bytes = kernel_buffers::test_account_used(principal);
    assert!(principal_bytes > 0);
    let reactor_bytes = kernel_buffers::test_reactor_used(principal);
    assert!(reactor_bytes > 0);
    let metadata_bytes = kernel_buffers::test_metadata_used(principal);
    assert!(metadata_bytes > 0);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let occupied = ADDRESS_SPACE_TABLE.lock().iter().filter(|entry| entry.is_some()).count();
    {
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let table = ADDRESS_SPACE_TABLE.lock();
        let _limits = DOMAIN_LIMITS.lock();
        let _usage = usage::DOMAIN_USAGE.lock();
        let _authority = DOMAIN_AUTHORITIES.lock();
        budget::tests::with_ledger_locked(|| {
            let allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
            usage::with_principal_history_locked(|| {
                let _heap = allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
                crate::klib::namespace_account_budget::with_locked(|| {
                    crate::klib::namespace_node_budget::with_locked(|| {
                        crate::klib::observer::account_budget::with_locked(|| {
                            crate::timers::account_budget::with_locked(|| drop(owner));
                        });
                    });
                });
            });
            assert_eq!(allocator.free_frames(), free);
            assert_eq!(table.iter().filter(|entry| entry.is_some()).count(), occupied);
        });
    }
    assert_eq!(crate::klib::namespace_account_budget::snapshot(), (accounts.0 + 3, accounts.1 + 3));
    assert_eq!(kernel_buffers::test_account_used(principal), principal_bytes);
    assert_eq!(kernel_buffers::test_reactor_used(principal), reactor_bytes);
    assert_eq!(kernel_buffers::test_metadata_used(principal), metadata_bytes);
    crate::logln!(
        "[memory sponsor registration retention] guarded abandonment retains one unpublished \
         root, capability/IPC namespaces, complete sponsors and policy/usage/authority metadata; \
         no ASID published; {} namespace + {} reactor account + {} metadata original principal \
         bytes retained",
        principal_bytes,
        reactor_bytes,
        metadata_bytes
    );
}

#[cfg(any(test, feature = "kernel_tests"))]
mod ipc_tests;

#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn test_ipc_preparation() {
    ipc_tests::run();
}
