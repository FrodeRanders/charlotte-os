//! Closing roots compose bounded IPC loan cleanup outside lifecycle/IPC.
//! Remaining memory/device cleanup retains its serialization; the detached
//! owner leases its slot through final invalidation/destruction.

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
    cleanup_started: bool,
    ipc_closed: bool,
}

#[must_use]
pub(crate) enum RetirementProgress {
    Pending(ClosingAddressSpace),
    Ready(RetiredAddressSpace),
}

#[must_use]
pub(crate) enum CloseProgress {
    Pending(ClosingAddressSpace),
    Complete,
}

impl ClosingAddressSpace {
    /// Caller must establish thread quiescence before staging root close.
    pub(crate) fn begin(handle: AddressSpaceHandle) -> Result<Self, AddressSpaceCloseError> {
        Self::begin_with(handle, |_, _| Ok(()))
    }

    /// Immediate close rejects busy roots before publishing a fence.
    pub(crate) fn begin_ready(handle: AddressSpaceHandle) -> Result<Self, AddressSpaceCloseError> {
        Self::begin_with(handle, super::AddressSpaceTable::prepare_retirement)
    }

    fn begin_with(
        handle: AddressSpaceHandle,
        prepare: impl FnOnce(&mut super::AddressSpaceTable, usize) -> Result<(), Error>,
    ) -> Result<Self, AddressSpaceCloseError> {
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
        if table.is_closing(handle.id()).unwrap() {
            return Err(AddressSpaceCloseError::CloseInProgress);
        }
        prepare(&mut table, handle.id()).map_err(|error| match error {
            Error::Leased => AddressSpaceCloseError::OperationsInFlight,
            error => close_error(error),
        })?;
        let slot = table.begin_close(handle.id(), handle.generation()).map_err(close_error)?;
        Ok(Self {
            handle,
            slot,
            cleanup_started: false,
            ipc_closed: false,
        })
    }

    pub(crate) fn poll(self) -> Result<CloseProgress, AddressSpaceCloseError> {
        self.poll_with(invalidate)
    }

    fn poll_with(
        self,
        invalidate: impl FnOnce(&AddressSpace, AddressSpaceHandle) -> bool,
    ) -> Result<CloseProgress, AddressSpaceCloseError> {
        match self.prepare_retirement()? {
            RetirementProgress::Pending(pending) => Ok(CloseProgress::Pending(pending)),
            RetirementProgress::Ready(retired) => {
                retired.release_with(invalidate)?;
                Ok(CloseProgress::Complete)
            }
        }
    }

    pub(crate) fn handle(&self) -> AddressSpaceHandle {
        self.handle
    }

    /// Peer admission is available only to a borrowed, started closing owner.
    /// Ordinary operations still reject both closing roots. No guard escapes.
    pub(crate) fn retain_peer(
        &self,
        handle: AddressSpaceHandle,
    ) -> Result<super::operation::AddressSpaceOperation, super::operation::OperationError> {
        assert!(self.cleanup_started && !self.ipc_closed);
        super::operation::AddressSpaceOperation::acquire_for_close(handle, &self.slot)
    }

    pub(crate) fn prepare_retirement(self) -> Result<RetirementProgress, AddressSpaceCloseError> {
        self.prepare_with_loan_cleanup(super::object::LoanRevocation::finish)
    }

    // Boot fixtures can reject a real prepared receipt and inspect the unlocked
    // interval. Production always uses LoanRevocation's physical cleanup.
    pub(crate) fn prepare_with_loan_cleanup(
        mut self,
        finish: impl FnMut(
            super::object::LoanRevocation,
        ) -> Result<(), super::object::MemoryObjectError>,
    ) -> Result<RetirementProgress, AddressSpaceCloseError> {
        if !self.start_cleanup()? {
            return Ok(RetirementProgress::Pending(self));
        }
        if !self.ipc_closed {
            if !crate::ipc::namespace_close::close_with(&self, finish)
                .map_err(|_| AddressSpaceCloseError::IpcCleanupFailed)?
            {
                return Ok(RetirementProgress::Pending(self));
            }
            self.ipc_closed = true;
        }
        let _lifecycle = super::ADDRESS_SPACE_LIFECYCLE.lock();
        {
            let mut table = ADDRESS_SPACE_TABLE.lock();
            match table.prepare_closing_retirement(&self.slot) {
                Err(Error::Leased) => return Ok(RetirementProgress::Pending(self)),
                Err(error) => return Err(close_error(error)),
                Ok(()) => {}
            }
            table.seal_close(&self.slot).map_err(close_error)?;
        }
        super::finish_user_address_space_cleanup(self.handle, self.slot)
            .map(RetirementProgress::Ready)
    }

    /// Establish logical admission/DMA fences once, after older leases drain.
    /// This separated phase also permits deterministic closing-peer fixtures.
    pub(crate) fn start_cleanup(&mut self) -> Result<bool, AddressSpaceCloseError> {
        if self.cleanup_started {
            return Ok(true);
        }
        let _lifecycle = super::ADDRESS_SPACE_LIFECYCLE.lock();
        {
            let mut table = ADDRESS_SPACE_TABLE.lock();
            match table.prepare_closing_retirement(&self.slot) {
                Err(Error::Leased) => return Ok(false),
                Err(error) => return Err(close_error(error)),
                Ok(()) => {}
            }
        }
        super::begin_user_address_space_cleanup(self.handle);
        crate::ipc::namespace_close::begin(self)
            .map_err(|_| AddressSpaceCloseError::IpcCleanupFailed)?;
        self.cleanup_started = true;
        Ok(true)
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

    pub(crate) fn release(self) -> Result<(), AddressSpaceCloseError> {
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
