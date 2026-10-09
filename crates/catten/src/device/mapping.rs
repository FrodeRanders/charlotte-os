//! Public DMA map/unmap/close lease their exact root and claim the capability;
//! backend maintenance separately owns the actual domain/engine and data pin.
use super::{
    detached_domain::{
        DetachedDomain,
        Maintenance,
    },
    *,
};
use crate::memory::object::{
    self,
    DmaPin,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Map,
    Rollback,
    Unmap,
}

#[must_use]
pub(super) struct PendingPin(DetachedDomain<Option<DmaPin>>);

impl PendingPin {
    pub(super) fn new(pin: Option<DmaPin>) -> Self {
        Self(DetachedDomain::new(pin))
    }

    pub(super) fn borrow(&mut self) -> &DmaPin {
        self.0.value_mut().as_ref().expect("missing DMA pin")
    }

    pub(super) fn take(&mut self) -> DmaPin {
        self.0.value_mut().take().expect("DMA pin consumed twice")
    }

    pub(super) fn retain(&mut self, pin: DmaPin) {
        assert!(self.0.value_mut().is_none(), "pending DMA pin replaced");
        *self.0.value_mut() = Some(pin);
    }

    pub(super) fn release(self) {
        if let Some(pin) = self.0.into_inner() {
            object::unpin_dma(pin);
        }
    }
}

#[must_use]
pub(super) struct MappingMaintenance<D, C> {
    pub(super) held: Maintenance<D, C>,
    pub(super) pending: PendingPin,
}

impl<D, C> MappingMaintenance<D, C> {
    pub(super) fn new(held: Maintenance<D, C>, pending: PendingPin) -> Self {
        Self {
            held,
            pending,
        }
    }
}

/// Field fallback retains the exact root. The registry's in-flight claim also
/// remains until explicit completion, including backend restoration and unpin.
#[must_use]
struct DmaOperation {
    root: core::mem::ManuallyDrop<Option<AddressSpaceOperation>>,
    asid: AddressSpaceId,
    cap: DeviceCap,
    id: u64,
}

impl DmaOperation {
    fn validate(asid: AddressSpaceId, cap: DeviceCap) -> Result<(), DeviceError> {
        let mut devices = DEVICES.lock();
        match lookup_mut(&mut devices, asid, cap)? {
            DeviceObject::DmaDomain {
                operation_in_flight: true,
                ..
            } => Err(DeviceError::OperationInFlight),
            DeviceObject::DmaDomain {
                ..
            } => Ok(()),
            _ => Err(DeviceError::WrongType),
        }
    }

    fn begin(asid: AddressSpaceId, cap: DeviceCap) -> Result<Self, DeviceError> {
        Self::validate(asid, cap)?;
        let root = if asid == crate::memory::KERNEL_ASID {
            None // Permanent kernel root, never a reusable user generation.
        } else {
            let handle = crate::memory::current_address_space_handle(asid)
                .ok_or(DeviceError::NamespaceRetired)?;
            Some(AddressSpaceOperation::acquire(handle).map_err(|error| match error {
                OperationError::Limit => DeviceError::ResourceLimit,
                _ => operation_error(error),
            })?)
        };
        let claim = (|| {
            let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
            let mut devices = DEVICES.lock();
            let DeviceObject::DmaDomain {
                id,
                operation_in_flight,
            } = lookup_mut(&mut devices, asid, cap)?
            else {
                return Err(DeviceError::WrongType);
            };
            if *operation_in_flight {
                return Err(DeviceError::OperationInFlight);
            }
            *operation_in_flight = true;
            Ok(*id)
        })();
        match claim {
            Ok(id) => Ok(Self {
                root: core::mem::ManuallyDrop::new(root),
                asid,
                cap,
                id,
            }),
            Err(error) => {
                if let Some(root) = root {
                    root.release().map_err(operation_error)?;
                }
                Err(error)
            }
        }
    }

    fn complete(self) -> Result<(), DeviceError> {
        {
            let mut devices = DEVICES.lock();
            let DeviceObject::DmaDomain {
                id,
                operation_in_flight,
            } = lookup_mut(&mut devices, self.asid, self.cap)?
            else {
                return Err(DeviceError::WrongType);
            };
            if *id != self.id || !*operation_in_flight {
                return Err(DeviceError::OperationInFlight);
            }
            *operation_in_flight = false;
        }
        if let Some(root) = core::mem::ManuallyDrop::into_inner(self.root) {
            root.release().map_err(operation_error)?;
        }
        Ok(())
    }

    /// Consume authority only after confirmed backend destruction. Keeping the
    /// original payload cell claimed avoids extracting/reallocating metadata
    /// on rejection and fences competing close/namespace cleanup on abandonment.
    fn complete_close(self) -> Result<(), DeviceError> {
        let entry = {
            let mut devices = DEVICES.lock();
            let DeviceObject::DmaDomain {
                id,
                operation_in_flight,
            } = lookup_mut(&mut devices, self.asid, self.cap)?
            else {
                return Err(DeviceError::WrongType);
            };
            if *id != self.id || !*operation_in_flight {
                return Err(DeviceError::OperationInFlight);
            }
            assert!(crate::capability::remove(
                self.asid,
                self.cap,
                crate::capability::ObjectKind::Device
            ));
            devices.get_mut(&self.asid).unwrap().caps.take(&self.cap).unwrap()
        };
        registry::release(entry);
        if let Some(root) = core::mem::ManuallyDrop::into_inner(self.root) {
            root.release().map_err(operation_error)?;
        }
        Ok(())
    }
}

pub(super) fn close_with(
    asid: AddressSpaceId,
    cap: DeviceCap,
    after_claim: impl FnOnce(),
    destroy: impl FnOnce(u64) -> Result<(), dma::Error>,
) -> Result<(), DeviceError> {
    let operation = DmaOperation::begin(asid, cap)?;
    after_claim();
    match destroy(operation.id) {
        Ok(()) => operation.complete_close(),
        Err(_) => {
            // A returned backend error has restored/retained its complete
            // registered owner. Preserve original authority/payload without
            // allocation, and finish this ordinary root lease. Frozen physical
            // backing remains terminal; this does not authorize physical retry.
            operation.complete()?;
            Err(DeviceError::DmaInvalid)
        }
    }
}

pub(super) fn with_operation<T>(
    asid: AddressSpaceId,
    cap: DeviceCap,
    work: impl FnOnce(u64) -> Result<T, dma::Error>,
) -> Result<T, DeviceError> {
    let operation = DmaOperation::begin(asid, cap)?;
    let result = work(operation.id).map_err(|_| DeviceError::DmaInvalid);
    // Backend errors return only after exact domain/engine restoration, with
    // uncertain data pins retained there. Abandonment never reaches completion.
    operation.complete()?;
    result
}

pub(super) fn test_admission() {
    tests::run();
}
mod tests;
