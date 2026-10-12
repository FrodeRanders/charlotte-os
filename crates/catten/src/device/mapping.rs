//! Public DMA map/unmap/close lease their exact root and claim the capability;
//! backend maintenance separately owns the actual domain/engine and data pin.
pub(super) use super::mapping_storage::PendingPin;
use super::{
    detached_domain::Maintenance,
    *,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Map,
    Rollback,
    Unmap,
}

/// Only rejection before taking the actual domain/engine is admission.
/// Errors after that boundary cannot authorize replay of an unmap attempt.
pub(crate) enum UnmapError {
    Admission(dma::Error),
    Completion(dma::Error),
}

impl UnmapError {
    pub(super) fn cause(self) -> dma::Error {
        match self {
            Self::Admission(error) | Self::Completion(error) => error,
        }
    }

    fn status(self) -> DeviceError {
        match self {
            Self::Admission(dma::Error::OperationInFlight) => DeviceError::OperationInFlight,
            _ => DeviceError::DmaInvalid,
        }
    }
}

/// Per-return diagnostics only. Backend rejection includes both pre-claim
/// admission and started maintenance; it never authorizes map replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MapError {
    Validation(dma::Error),
    Admission(DeviceError),
    Backend(dma::Error),
    Completion(DeviceError),
}

impl MapError {
    pub(crate) fn diagnostic(self) -> u64 {
        use catten_syscall::{
            dma_map_cause as cause,
            dma_map_phase as phase,
        };
        let backend_cause = |error| match error {
            dma::Error::Unsupported => cause::UNSUPPORTED,
            dma::Error::InvalidStream => cause::INVALID_STREAM,
            dma::Error::StreamInUse => cause::STREAM_IN_USE,
            dma::Error::InvalidDirection => cause::INVALID_DIRECTION,
            dma::Error::Memory => cause::MEMORY,
            dma::Error::OutOfIova => cause::OUT_OF_IOVA,
            dma::Error::MapFailed => cause::MAP_FAILED,
            dma::Error::AlreadyMapped => cause::ALREADY_MAPPED,
            dma::Error::UnknownDomain => cause::UNKNOWN_DOMAIN,
            dma::Error::UnknownMapping => cause::UNKNOWN_MAPPING,
            dma::Error::HardwareTimeout => cause::HARDWARE_TIMEOUT,
            dma::Error::OperationInFlight => cause::OPERATION_IN_FLIGHT,
            dma::Error::WaitRegistrationFailed => cause::WAIT_REGISTRATION_FAILED,
        };
        let (stage, reason) = match self {
            Self::Validation(error) => (phase::VALIDATION, backend_cause(error)),
            Self::Admission(_) => (phase::ADMISSION, cause::NONE),
            Self::Backend(error) => (phase::BACKEND, backend_cause(error)),
            Self::Completion(_) => (phase::COMPLETION, cause::NONE),
        };
        stage | (reason << 8)
    }

    pub(crate) fn status(self) -> DeviceError {
        match self {
            Self::Admission(error) | Self::Completion(error) => error,
            Self::Validation(_) | Self::Backend(_) => DeviceError::DmaInvalid,
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
    payload:
        Option<crate::klib::collections::retirement_list::RetiredEntry<(DeviceCap, DeviceObject)>>,
    authority: Option<crate::capability::RetiredRecord>,
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
                payload: None,
                authority: None,
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
    fn complete_close(mut self) -> Result<(), DeviceError> {
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
            self.authority = Some(
                crate::capability::detach(
                    self.asid,
                    self.cap,
                    crate::capability::ObjectKind::Device,
                )
                .expect("confirmed DMA close authority absent"),
            );
            self.payload = Some(devices.get_mut(&self.asid).unwrap().caps.take(&self.cap).unwrap());
        }
        registry::release(self.payload.take().unwrap());
        registry::release_authority(self.authority.take().unwrap());
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

pub(super) fn map_with(
    asid: AddressSpaceId,
    cap: DeviceCap,
    work: impl FnOnce(u64) -> Result<u64, dma::Error>,
) -> Result<u64, MapError> {
    let operation = DmaOperation::begin(asid, cap).map_err(MapError::Admission)?;
    let result = work(operation.id).map_err(MapError::Backend);
    // Preserve the backend rejection even if finishing the public lease also
    // fails. Neither diagnostic is a cleanup receipt or permission to retry.
    let completion = operation.complete().map_err(MapError::Completion);
    match result {
        Err(error) => Err(error),
        Ok(iova) => completion.map(|()| iova),
    }
}

pub(super) fn unmap_with(
    asid: AddressSpaceId,
    cap: DeviceCap,
    work: impl FnOnce(u64) -> Result<(), UnmapError>,
) -> Result<(), DeviceError> {
    let operation = DmaOperation::begin(asid, cap)?;
    let result = work(operation.id).map_err(UnmapError::status);
    // Completion rejection has already restored exact state and quarantined
    // uncertain pins. Finishing the public claim grants no cleanup retry.
    // A final claim/root-completion rejection is also after backend work;
    // never let its own busy-looking error escape as pre-work admission.
    operation.complete().map_err(|_| DeviceError::DmaInvalid)?;
    result
}

pub(super) fn test_admission() {
    tests::run();
}
mod tests;
