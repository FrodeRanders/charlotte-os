//! Shared admission policy/storage; DMA keeps its typed hardware obligation.
use core::mem::ManuallyDrop;

use super::*;

pub(super) struct GrantAdmission {
    pub(super) resources: ManuallyDrop<Resources>,
}
pub(super) struct Resources {
    owner: AddressSpaceId,
    pub(super) address_space: Option<AddressSpaceOperation>,
    pub(super) reservation: Option<crate::capability::Reservation>,
    storage: Option<registry::PreparedStorage>,
    authority: Option<crate::capability::PreparedReservation>,
}
impl GrantAdmission {
    pub(super) fn new(owner: AddressSpaceId) -> Result<Self, DeviceError> {
        let address_space = if owner == crate::memory::KERNEL_ASID {
            None
        } else {
            let handle = crate::memory::current_address_space_handle(owner)
                .ok_or(DeviceError::NamespaceRetired)?;
            Some(AddressSpaceOperation::acquire(handle).map_err(operation_error)?)
        };
        let mut admission = Self {
            resources: ManuallyDrop::new(Resources {
                owner,
                address_space,
                reservation: None,
                storage: None,
                authority: None,
            }),
        };
        match registry::PreparedStorage::try_new() {
            Ok(storage) => admission.resources.storage = Some(storage),
            Err(error) => {
                admission.finish()?;
                return Err(error);
            }
        }
        registry::tests::authority_boundary(false);
        match crate::capability::PreparedReservation::try_new(
            owner,
            crate::capability::ObjectKind::Device,
            admission.resources.address_space.as_ref().map(AddressSpaceOperation::handle),
        ) {
            Ok(authority) => admission.resources.authority = Some(authority),
            Err(error) => {
                admission.finish()?;
                return Err(admission_error(error));
            }
        }
        Ok(admission)
    }

    pub(super) fn validate_publication(&self) -> Result<(), DeviceError> {
        if let Some(operation) = self.resources.address_space.as_ref() {
            let handle = operation.handle();
            let table = crate::memory::ADDRESS_SPACE_TABLE.lock();
            if table.generation(handle.id()).ok() != Some(handle.generation()) {
                return Err(DeviceError::NamespaceRetired);
            }
            if table.is_closing(handle.id()).unwrap_or(true) {
                return Err(DeviceError::AddressSpaceClosing);
            }
        }
        Ok(())
    }

    pub(super) fn reserve(
        &mut self,
        lifecycle: &crate::capability::LifecycleGuard<'_>,
    ) -> Result<(), DeviceError> {
        self.validate_publication()?;
        assert!(self.resources.reservation.is_none());
        self.resources.reservation = Some(
            self.resources
                .authority
                .as_mut()
                .unwrap()
                .reserve_in_lifecycle(lifecycle)
                .map_err(admission_error)?,
        );
        Ok(())
    }

    pub(super) fn publish(
        &mut self,
        devices: &mut Map<AddressSpaceId, AsDeviceCaps>,
        object: DeviceObject,
    ) -> Result<DeviceCap, DeviceError> {
        let resources = &mut *self.resources;
        let owner = resources.owner;
        let reservation = resources.reservation.as_mut().unwrap();
        let cap = reservation.identity();
        assert!(
            devices.get(&owner).is_none_or(|caps| !caps.caps.contains_key(&cap)),
            "device capability replaced"
        );
        crate::capability::publish_batch(&mut [crate::capability::Publication {
            destination: reservation,
            source: None,
        }])
        .map_err(admission_error)?;
        resources.storage.as_mut().unwrap().publish(devices, owner, cap, object);
        Ok(cap)
    }

    /// Ordinary explicit completion leaves every local publication guard first.
    /// DMA may call this only after confirmed hardware rollback/activation.
    pub(super) fn finish(&mut self) -> Result<(), DeviceError> {
        if let Some(storage) = self.resources.storage.take() {
            storage.release();
        }
        if let Some(authority) = self.resources.authority.take() {
            registry::tests::authority_boundary(true);
            authority.finish();
        }
        if self.resources.reservation.is_some() {
            registry::tests::authority_boundary(true);
        }
        drop(self.resources.reservation.take());
        if let Some(root) = self.resources.address_space.take() {
            root.release().map_err(operation_error)?;
        }
        Ok(())
    }
}

pub(super) fn plain(
    owner: AddressSpaceId,
    publish: impl FnOnce(&mut GrantAdmission) -> Result<DeviceCap, DeviceError>,
) -> Result<DeviceCap, DeviceError> {
    let mut admission = GrantAdmission::new(owner)?;
    let result = publish(&mut admission);
    admission.finish()?;
    result
}
