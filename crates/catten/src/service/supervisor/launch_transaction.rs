//! Complete unstarted launch ownership. Only explicit, post-guard rollback
//! enters physical cleanup; abandonment retains every field without retry.
use core::{
    mem::ManuallyDrop,
    ops::{
        Deref,
        DerefMut,
    },
};

use super::{
    ProfileLaunchError,
    loader,
};
use crate::memory::{
    self,
    AddressSpaceHandle,
    KERNEL_ASID,
    operation::AddressSpaceOperation,
};

#[must_use]
pub(super) struct DomainLaunchTransaction(ManuallyDrop<Resources>);
pub(super) struct Resources {
    loaded: Option<loader::LoadedDomain>,
    pub(super) kernel_profile: Option<memory::object::MemoryObjectCap>,
    root: Option<AddressSpaceOperation>,
    sources: [Option<AddressSpaceOperation>; 3],
}
impl Deref for DomainLaunchTransaction {
    type Target = Resources;

    fn deref(&self) -> &Resources {
        &self.0
    }
}
impl DerefMut for DomainLaunchTransaction {
    fn deref_mut(&mut self) -> &mut Resources {
        &mut self.0
    }
}
impl DomainLaunchTransaction {
    pub(super) fn new(loaded: loader::LoadedDomain) -> Result<Self, ProfileLaunchError> {
        let handle = loaded.address_space;
        let mut owner = Self(ManuallyDrop::new(Resources {
            loaded: Some(loaded),
            kernel_profile: None,
            root: None,
            sources: [None, None, None],
        }));
        match AddressSpaceOperation::acquire(handle) {
            Ok(root) => owner.root = Some(root),
            Err(_) => return owner.abort(ProfileLaunchError::RootAdmission),
        }
        Ok(owner)
    }

    pub(super) fn loaded(&self) -> &loader::LoadedDomain {
        self.loaded.as_ref().expect("launch transaction consumed")
    }

    /// Call before delegating from a captured service identity, never beneath
    /// IPC/lifecycle. Duplicate identities use the already retained lease.
    pub(super) fn retain_source(
        &mut self,
        handle: AddressSpaceHandle,
    ) -> Result<(), ProfileLaunchError> {
        if handle.id() == KERNEL_ASID
            || self.loaded().address_space == handle
            || self.sources.iter().flatten().any(|root| root.handle() == handle)
        {
            return Ok(());
        }
        let slot = self
            .sources
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ProfileLaunchError::RootAdmission)?;
        *slot = Some(
            AddressSpaceOperation::acquire(handle)
                .map_err(|_| ProfileLaunchError::RootAdmission)?,
        );
        Ok(())
    }

    fn complete_sources(&mut self) -> Result<(), ProfileLaunchError> {
        for source in &mut self.sources {
            if let Some(source) = source.take() {
                source.release().map_err(|_| ProfileLaunchError::RootCompletion)?;
            }
        }
        Ok(())
    }

    /// Each physical operation is attempted once. An error retains all
    /// remaining fields and charges; it never enters a best-effort second pass.
    pub(super) fn cancel(mut self) -> Result<(), ProfileLaunchError> {
        if let Some(profile) = self.kernel_profile {
            memory::object::close_cap(KERNEL_ASID, profile)
                .map_err(ProfileLaunchError::RollbackMemory)?;
            self.kernel_profile = None;
        }
        if let Some(root) = self.root.take() {
            root.release().map_err(|_| ProfileLaunchError::RootCompletion)?;
        }
        if let Some(loaded) = self.loaded.as_ref() {
            memory::close_user_address_space_handle(loaded.address_space)
                .map_err(ProfileLaunchError::RollbackDomain)?;
            self.loaded = None;
        }
        self.complete_sources()?;
        Ok(())
    }

    pub(super) fn abort<T>(self, error: ProfileLaunchError) -> Result<T, ProfileLaunchError> {
        self.cancel()?;
        Err(error)
    }

    /// The initial thread now owns its stack/root admission. Its complete
    /// launch state must be published before ending these preparation leases.
    pub(super) fn finish(mut self) -> loader::LoadedDomain {
        assert!(self.kernel_profile.is_none());
        self.complete_sources().expect("launch source completion failed");
        self.root
            .take()
            .expect("launch root missing")
            .release()
            .expect("launch root completion failed");
        self.loaded.take().expect("launch transaction consumed")
    }
}

#[cfg(feature = "kernel_tests")]
pub(super) mod tests;
