//! Live-generation retention foundation. This does not itself permit releasing
//! a mapping's lifecycle/IPC guards: backing, scratch and authority need their
//! own retained completion owners as well.

use super::{
    ADDRESS_SPACE_LIFECYCLE,
    ADDRESS_SPACE_TABLE,
    AddressSpaceHandle,
    KERNEL_ASID,
};
use crate::klib::collections::id_table::{
    Error,
    SlotLease,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationError {
    KernelAddressSpace,
    AddressSpaceMissing,
    StaleHandle,
    Limit,
    WrongLease,
    Closing,
}

/// Retains the exact live root/slot and hence its hardware tag and accounts.
/// No address-space pointer escapes the table guard. Drop deliberately retains
/// the count; an unfinished operation cannot silently authorize root teardown.
#[must_use]
pub(crate) struct AddressSpaceOperation {
    handle: AddressSpaceHandle,
    lease: SlotLease,
}

impl AddressSpaceOperation {
    /// Only namespace retirement uses this admission. The borrowed closing
    /// owner and its peer must both precede sealed backing teardown.
    pub(super) fn acquire_for_close(
        handle: AddressSpaceHandle,
        owner: &crate::klib::collections::id_table::ClosingSlot,
    ) -> Result<Self, OperationError> {
        if handle.id() == KERNEL_ASID {
            return Err(OperationError::KernelAddressSpace);
        }
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let mut table = ADDRESS_SPACE_TABLE.lock();
        match table.generation(handle.id()) {
            Ok(generation) if generation == handle.generation() => {}
            Ok(_) => return Err(OperationError::StaleHandle),
            Err(_) => return Err(OperationError::AddressSpaceMissing),
        }
        let lease =
            table.lease_for_close(owner, handle.id(), handle.generation()).map_err(|error| {
                match error {
                    Error::LeaseLimit => OperationError::Limit,
                    Error::Closing => OperationError::Closing,
                    _ => OperationError::WrongLease,
                }
            })?;
        Ok(Self {
            handle,
            lease,
        })
    }

    /// Acquire lifecycle before the table, never under an IPC/device registry.
    /// This serializes admission with the complete close preflight/cleanup.
    pub(crate) fn acquire(handle: AddressSpaceHandle) -> Result<Self, OperationError> {
        if handle.id() == KERNEL_ASID {
            return Err(OperationError::KernelAddressSpace);
        }
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let mut table = ADDRESS_SPACE_TABLE.lock();
        match table.generation(handle.id()) {
            Ok(generation) if generation == handle.generation() => {}
            Ok(_) => return Err(OperationError::StaleHandle),
            Err(_) => return Err(OperationError::AddressSpaceMissing),
        }
        let lease = table.lease(handle.id(), handle.generation()).map_err(|error| match error {
            Error::LeaseLimit => OperationError::Limit,
            Error::Closing => OperationError::Closing,
            _ => OperationError::WrongLease,
        })?;
        Ok(Self {
            handle,
            lease,
        })
    }

    pub(crate) fn handle(&self) -> AddressSpaceHandle {
        self.handle
    }

    /// Caller proves its operation has terminated before consuming this owner.
    /// No allocation, invalidation, subsystem retirement or resource destructor
    /// runs here. The original table verifies identity/generation before release.
    pub(crate) fn release(self) -> Result<(), OperationError> {
        ADDRESS_SPACE_TABLE.lock().finish_lease(self.lease).map_err(|_| OperationError::WrongLease)
    }
}
