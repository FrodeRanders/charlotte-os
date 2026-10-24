//! Namespace construction belongs to root registration. Publication only
//! relinks prepared storage, with an exact-root lease across the unlocked gap.

use core::{
    alloc::AllocError,
    mem::ManuallyDrop,
};

use super::*;
use crate::{
    klib::collections::retirement_list::PreparedEntry,
    memory::{
        AddressSpaceHandle,
        KERNEL_ASID,
        operation::AddressSpaceOperation,
    },
};

struct Resources {
    node: Option<PreparedEntry<(AddressSpaceId, AsIpcCaps)>>,
    endpoints: Option<Arc<budget::DomainBudget>>,
    records: Option<Arc<record_budget::DomainBudget>>,
}

/// Partial preparation and its parent root transaction retain every field on
/// abandonment. Ordinary rejection explicitly cancels after local guards leave.
#[must_use]
pub(crate) struct PreparingNamespace(ManuallyDrop<Resources>);

impl PreparingNamespace {
    pub(crate) fn try_new() -> Result<Self, AllocError> {
        Self::prepare(|_| Ok(()))
    }

    fn prepare(mut before: impl FnMut(u8) -> Result<(), AllocError>) -> Result<Self, AllocError> {
        let mut owner = Self(ManuallyDrop::new(Resources {
            node: None,
            endpoints: None,
            records: None,
        }));
        let result = (|| {
            before(1)?;
            owner.0.node = Some(PreparedEntry::try_new()?);
            before(2)?;
            owner.0.endpoints = Some(budget::DomainBudget::try_new()?);
            before(3)?;
            owner.0.records = Some(record_budget::DomainBudget::try_new()?);
            before(4)
        })();
        if let Err(error) = result {
            owner.cancel_unpublished();
            return Err(error);
        }
        Ok(owner)
    }

    /// No lifecycle/table guard is held. The containing registration owns the
    /// lease until publication and all unused preparation have finished.
    pub(crate) fn publish(self, root: &AddressSpaceOperation) {
        let handle = root.handle();
        self.publish_into(&mut IPC.write(), handle.id(), Some(handle));
    }

    /// Mandatory BSP initialization precedes schedulers/APs. The kernel root
    /// never retires; runtime root construction uses the leased entry point.
    pub(crate) fn publish_kernel(self, handle: AddressSpaceHandle) {
        assert_eq!(handle.id(), KERNEL_ASID);
        self.publish_into(&mut IPC.write(), KERNEL_ASID, Some(handle));
    }

    fn publish_into(
        mut self,
        ipc: &mut IpcRegistry,
        id: AddressSpaceId,
        handle: Option<AddressSpaceHandle>,
    ) {
        assert!(!ipc.caps.contains_key(&id), "IPC namespace survived ASID teardown");
        ipc.caps.insert(
            self.0.node.take().unwrap(),
            id,
            AsIpcCaps {
                caps: BTreeMap::new(),
                address_space: handle,
                endpoint_budget: self.0.endpoints.take().unwrap(),
                record_budget: self.0.records.take().unwrap(),
            },
        );
    }

    pub(crate) fn cancel_unpublished(mut self) {
        drop(self.0.records.take());
        drop(self.0.endpoints.take());
        drop(self.0.node.take());
    }

    pub(crate) fn test_reject_at(reject: u8) -> Result<Self, AllocError> {
        Self::prepare(|stage| {
            assert!(IPC.try_write().is_some());
            assert!(crate::memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
            assert!(crate::memory::ADDRESS_SPACE_TABLE.try_lock().is_some());
            assert!(
                crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.try_lock().is_some()
            );
            if stage == reject {
                Err(AllocError)
            } else {
                Ok(())
            }
        })
    }
}

/// Synthetic IDs occupy the top half of usize, beyond any Vec-backed root
/// table's addressable slots. Only serialized kernel fixtures use this helper.
pub(crate) const fn fixture_id(id: usize) -> AddressSpaceId {
    id | (1usize << (usize::BITS - 1))
}

/// Explicit fixture preparation runs outside subsystem/lifecycle guards. It
/// grants no trusted classification and is never a runtime admission fallback.
pub(crate) fn prepare_fixture(id: AddressSpaceId) {
    assert_eq!(fixture_id(id), id, "fixture namespace overlaps real root slots");
    assert!(crate::memory::current_address_space_handle(id).is_none());
    {
        let ipc = IPC.read();
        if let Some(namespace) = ipc.caps.get(&id) {
            assert!(namespace.address_space.is_none());
            assert!(namespace.record_budget.accepting());
            return;
        }
    }
    let prepared = PreparingNamespace::try_new().expect("fixture IPC namespace preparation failed");
    let mut ipc = IPC.write();
    if ipc.caps.contains_key(&id) {
        assert!(ipc.caps[&id].address_space.is_none());
        drop(ipc);
        prepared.cancel_unpublished();
    } else {
        prepared.publish_into(&mut ipc, id, None);
    }
}

mod tests;

pub(crate) fn test_preparation() {
    tests::run();
}
