//! Final private-root retirement. Logical subsystem cleanup still requires
//! lifecycle serialization; the detached owner then leases its software slot
//! and owns backing through lock-free final invalidation/destruction.

use super::{
    ADDRESS_SPACE_TABLE,
    AddressSpace,
    AddressSpaceCloseError,
    AddressSpaceHandle,
};
use crate::klib::collections::id_table::{
    ClosingSlot,
    Error,
    RetiredEntry,
};

/// A staged close owns the operation-admission fence while older leases drain.
/// Drop retains the fence/root; it cannot cancel closing under unknown guards.
#[must_use]
pub(crate) struct ClosingAddressSpace {
    handle: AddressSpaceHandle,
    slot: ClosingSlot,
}

#[must_use]
pub(crate) enum CloseProgress {
    Pending(ClosingAddressSpace),
    Complete,
}

impl ClosingAddressSpace {
    /// Caller must establish thread quiescence before staging root close.
    pub(crate) fn begin(handle: AddressSpaceHandle) -> Result<Self, AddressSpaceCloseError> {
        if handle.id() == super::KERNEL_ASID {
            return Err(AddressSpaceCloseError::KernelAddressSpace);
        }
        let _lifecycle = super::ADDRESS_SPACE_LIFECYCLE.lock();
        let mut table = ADDRESS_SPACE_TABLE.lock();
        match table.generation(handle.id()) {
            Ok(generation) if generation == handle.generation() => {}
            Ok(_) => return Err(AddressSpaceCloseError::StaleHandle),
            Err(_) => return Err(AddressSpaceCloseError::AddressSpaceMissing),
        }
        let slot = table.begin_close(handle.id(), handle.generation()).map_err(close_error)?;
        Ok(Self {
            handle,
            slot,
        })
    }

    pub(crate) fn poll(self) -> Result<CloseProgress, AddressSpaceCloseError> {
        self.poll_with(invalidate)
    }

    fn poll_with(
        self,
        invalidate: impl FnOnce(&AddressSpace, AddressSpaceHandle) -> bool,
    ) -> Result<CloseProgress, AddressSpaceCloseError> {
        let retired = {
            let _lifecycle = super::ADDRESS_SPACE_LIFECYCLE.lock();
            {
                let mut table = ADDRESS_SPACE_TABLE.lock();
                match table.prepare_closing_retirement(&self.slot) {
                    Err(Error::Leased) => return Ok(CloseProgress::Pending(self)),
                    Err(error) => return Err(close_error(error)),
                    Ok(()) => {}
                }
            }
            super::close_user_address_space_after_preflight(self.handle, self.slot)?
        };
        retired.release_with(invalidate)?;
        Ok(CloseProgress::Complete)
    }

    /// Bounded polling/sleep happens only after this owner's lifecycle/table
    /// guards are gone. Timeout drops the request into retained closing state;
    /// it does not refund backing, clear the fence, or pretend close completed.
    pub(crate) fn wait(mut self, timeout_ms: u64) -> Result<(), AddressSpaceCloseError> {
        let deadline = crate::cpu::scheduler::monotonic_millis().saturating_add(timeout_ms);
        loop {
            match self.poll()? {
                CloseProgress::Complete => return Ok(()),
                CloseProgress::Pending(pending) => {
                    self = pending;
                    if crate::cpu::scheduler::monotonic_millis() >= deadline {
                        return Err(AddressSpaceCloseError::OperationDrainTimedOut);
                    }
                    crate::cpu::scheduler::sleep_millis(1);
                }
            }
        }
    }
}

fn close_error(error: Error) -> AddressSpaceCloseError {
    match error {
        Error::Closing => AddressSpaceCloseError::CloseInProgress,
        Error::IdNotActive => AddressSpaceCloseError::AddressSpaceMissing,
        _ => AddressSpaceCloseError::RetirementMetadataAllocationFailed,
    }
}

#[must_use]
pub(crate) struct RetiredAddressSpace {
    handle: AddressSpaceHandle,
    entry: RetiredEntry<AddressSpace>,
}

impl RetiredAddressSpace {
    pub(super) fn new(handle: AddressSpaceHandle, entry: RetiredEntry<AddressSpace>) -> Self {
        Self {
            handle,
            entry,
        }
    }

    pub(super) fn release(self) -> Result<(), AddressSpaceCloseError> {
        self.release_with(invalidate)
    }

    fn release_with(
        self,
        invalidate: impl FnOnce(&AddressSpace, AddressSpaceHandle) -> bool,
    ) -> Result<(), AddressSpaceCloseError> {
        if !invalidate(self.entry.value(), self.handle) {
            crate::early_logln!(
                "[memory] quarantined address-space root asid={} generation={}",
                self.handle.id(),
                self.handle.generation()
            );
            return Err(AddressSpaceCloseError::QuiescenceFailed);
        }
        // Quiescence precedes all root/data/table destruction and account
        // refunds. No masking guard is held during AddressSpace::drop.
        let slot = self.entry.release_value();
        super::budget::forget(self.handle);
        ADDRESS_SPACE_TABLE
            .lock()
            .finish_retirement(slot)
            .expect("retired address-space slot identity lost");
        Ok(())
    }
}

fn invalidate(space: &AddressSpace, handle: AddressSpaceHandle) -> bool {
    // This is deliberately resolved from the owning root, not the table: the
    // software entry is detached and its hardware tag is still privately held.
    #[cfg(target_arch = "aarch64")]
    {
        let _ = handle;
        if space.hw_asid() != 0 {
            crate::cpu::isa::memory::tlb::inval_hardware_asid(space.hw_asid());
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        let _ = space;
        // No PCID: the completed rendezvous flushes all non-global entries.
        crate::cpu::isa::memory::tlb::inval_asid(handle.id());
    }
    true
}

pub(crate) mod tests;
