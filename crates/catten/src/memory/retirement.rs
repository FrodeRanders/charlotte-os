//! Final private-root retirement. Logical subsystem cleanup still requires
//! lifecycle serialization; the detached owner then leases its software slot
//! and owns backing through lock-free final invalidation/destruction.

use super::{
    ADDRESS_SPACE_TABLE,
    AddressSpace,
    AddressSpaceCloseError,
    AddressSpaceHandle,
};
use crate::klib::collections::id_table::RetiredEntry;

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
