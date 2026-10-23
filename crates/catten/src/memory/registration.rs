//! One owner retains an unpublished root and all prepared publication metadata.
use core::mem::ManuallyDrop;

use super::*;

pub(super) mod metadata;

struct Resources {
    root: Option<AddressSpace>,
    namespace: Option<crate::capability::PreparingNamespace>,
    account: Option<budget::PreparingAccount>,
    metadata: Option<metadata::PreparingMetadata>,
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
        }))
    }

    fn prepare(
        &mut self,
        account: impl FnOnce() -> Result<budget::PreparingAccount, core::alloc::AllocError>,
        metadata: impl FnOnce() -> Result<metadata::PreparingMetadata, core::alloc::AllocError>,
    ) -> Result<(), AddressSpaceRegistrationError> {
        self.0.namespace = Some(
            crate::capability::prepare_namespace()
                .map_err(|_| AddressSpaceRegistrationError::CapabilityNamespaceAllocationFailed)?,
        );
        self.0.account = Some(
            account().map_err(|_| AddressSpaceRegistrationError::MemorySponsorAllocationFailed)?,
        );
        self.0.metadata = Some(
            metadata()
                .map_err(|_| AddressSpaceRegistrationError::DomainMetadataAllocationFailed)?,
        );
        Ok(())
    }

    fn finish(mut self, reject: impl FnOnce(AddressSpace)) {
        if let Some(metadata) = self.0.metadata.take() {
            metadata.cancel_unpublished();
        }
        if let Some(account) = self.0.account.take() {
            account.cancel_unpublished();
        }
        if let Some(namespace) = self.0.namespace.take() {
            namespace.cancel_unpublished();
        }
        if let Some(root) = self.0.root.take() {
            reject(root);
        }
    }
}

pub(super) fn register(
    root: AddressSpace,
    publish: impl FnOnce(
        &mut IdTable<AddressSpace>,
        AddressSpace,
    )
        -> Result<usize, (AddressSpace, crate::klib::collections::id_table::Error)>,
    reject: impl FnOnce(AddressSpace),
    account: impl FnOnce() -> Result<budget::PreparingAccount, core::alloc::AllocError>,
    metadata: impl FnOnce() -> Result<metadata::PreparingMetadata, core::alloc::AllocError>,
) -> Result<AddressSpaceHandle, AddressSpaceRegistrationError> {
    let mut owner = PreparingRegistration::capture(root);
    if let Err(error) = owner.prepare(account, metadata) {
        owner.finish(reject);
        return Err(error);
    }
    let lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
    if let Err(error) = prepare_user_address_space(owner.0.root.as_mut().unwrap()) {
        drop(lifecycle);
        owner.finish(reject);
        return Err(error);
    }
    let mut table = ADDRESS_SPACE_TABLE.lock();
    let id = match publish(&mut table, owner.0.root.take().unwrap()) {
        Ok(id) => id,
        Err((root, _)) => {
            owner.0.root = Some(root);
            drop(table);
            drop(lifecycle);
            owner.finish(reject);
            return Err(AddressSpaceRegistrationError::TableAllocationFailed);
        }
    };
    debug_assert_ne!(id, KERNEL_ASID);
    let handle = AddressSpaceHandle {
        id,
        generation: table.generation(id).expect("new address space missing generation"),
    };
    owner.0.account.take().unwrap().publish(handle);
    owner.0.namespace.take().unwrap().publish(handle);
    owner.0.metadata.take().unwrap().publish(handle);
    drop(table);
    drop(lifecycle);
    owner.finish(reject);
    Ok(handle)
}

pub(super) fn test_abandonment() {
    let root = AddressSpace::try_new_user().unwrap();
    let mut owner = PreparingRegistration::capture(root);
    owner
        .prepare(budget::PreparingAccount::try_new, || metadata::PreparingMetadata::try_new(false))
        .unwrap();
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
                drop(owner);
            });
            assert_eq!(allocator.free_frames(), free);
            assert_eq!(table.iter().filter(|entry| entry.is_some()).count(), occupied);
        });
    }
    crate::logln!(
        "[memory sponsor registration retention] guarded abandonment retains one unpublished \
         root, capability namespace, complete sponsor and policy/usage/authority metadata; no \
         ASID published"
    );
}
