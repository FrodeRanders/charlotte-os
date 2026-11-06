//! Namespace construction belongs to root registration. Publication only
//! relinks prepared storage, with an exact-root lease across the unlocked gap.

use core::{
    alloc::AllocError,
    mem::ManuallyDrop,
};

use super::*;
use crate::memory::{
    AddressSpaceHandle,
    KERNEL_ASID,
    operation::AddressSpaceOperation,
};

struct Resources {
    node: Option<namespace_nodes::Prepared<(AddressSpaceId, AsIpcCaps)>>,
    endpoints: Option<budget::AccountRef>,
    records: Option<record_budget::AccountRef>,
}

/// Partial preparation and its parent root transaction retain every field on
/// abandonment. Ordinary rejection explicitly cancels after local guards leave.
#[must_use]
pub(crate) struct PreparingNamespace(ManuallyDrop<Resources>);

impl PreparingNamespace {
    #[cfg(any(test, feature = "kernel_tests"))]
    pub(crate) fn try_new() -> Result<Self, AllocError> {
        Self::try_new_with_backing_class(false)
    }

    pub(crate) fn try_new_with_backing_class(
        storage: impl Into<crate::memory::kernel_buffers::CapturedStorage>,
    ) -> Result<Self, AllocError> {
        Self::prepare(storage.into(), |_| Ok(()))
    }

    fn prepare(
        storage: crate::memory::kernel_buffers::CapturedStorage,
        mut before: impl FnMut(u8) -> Result<(), AllocError>,
    ) -> Result<Self, AllocError> {
        let mut owner = Self(ManuallyDrop::new(Resources {
            node: None,
            endpoints: None,
            records: None,
        }));
        let result = (|| {
            before(1)?;
            owner.0.node = Some(namespace_nodes::prepare(storage.platform())?);
            before(2)?;
            owner.0.endpoints = Some(budget::DomainBudget::try_new(storage)?);
            before(3)?;
            owner.0.records = Some(record_budget::DomainBudget::try_new(storage)?);
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
                caps: AdmittedMap::new(),
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

    #[cfg(any(test, feature = "kernel_tests"))]
    pub(crate) fn test_reject_at(reject: u8) -> Result<Self, AllocError> {
        Self::prepare(false.into(), |stage| {
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
#[cfg(any(test, feature = "kernel_tests"))]
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

#[cfg(any(test, feature = "kernel_tests"))]
mod tests;

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn test_preparation() {
    tests::run();
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn assert_account_backing_denied(
    storage: impl Into<crate::memory::kernel_buffers::CapturedStorage> + Copy,
) {
    assert!(budget::DomainBudget::try_new(storage).is_err());
    assert!(record_budget::DomainBudget::try_new(storage).is_err());
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn assert_ordinary_backing_fixture(root: AddressSpaceHandle) {
    let endpoints = budget::node_used();
    let records = record_budget::node_used();
    let endpoint = endpoint_create(root.id(), 1, 1, 4).unwrap();
    let connection = connection_mint(root.id(), endpoint, ConnectionRights::SEND).unwrap();
    assert_eq!(budget::node_used().0[0], endpoints.0[0] + 1);
    assert_eq!(budget::node_used().1[0], endpoints.1[0] + 1);
    assert_eq!(record_budget::node_used().0[0], records.0[0] + 1);
    assert_eq!(record_budget::node_used().1[0], records.1[0] + 1);
    close_cap(root.id(), connection).unwrap();
    close_cap(root.id(), endpoint).unwrap();
    assert_eq!(budget::node_used(), endpoints);
    assert_eq!(record_budget::node_used(), records);
}

#[cfg(feature = "kernel_tests")]
pub(crate) fn account_bytes_for_test(
    storage: crate::memory::kernel_buffers::CapturedStorage,
    principal: u64,
) -> [u64; 2] {
    let before = crate::memory::kernel_buffers::test_account_used(principal);
    let endpoint = budget::DomainBudget::try_new(storage).unwrap();
    let first = crate::memory::kernel_buffers::test_account_used(principal) - before;
    drop(endpoint);
    assert_eq!(crate::memory::kernel_buffers::test_account_used(principal), before);
    let records = record_budget::DomainBudget::try_new(storage).unwrap();
    let second = crate::memory::kernel_buffers::test_account_used(principal) - before;
    drop(records);
    assert_eq!(crate::memory::kernel_buffers::test_account_used(principal), before);
    [first, second]
}
#[cfg(feature = "kernel_tests")]
pub(crate) fn account_weaks_for_test(asid: usize) -> (impl Fn() -> bool, impl Fn() -> bool) {
    let ipc = IPC.read();
    let endpoints = alloc::sync::Arc::downgrade(&ipc.caps.get(&asid).unwrap().endpoint_budget);
    let records = alloc::sync::Arc::downgrade(&ipc.caps.get(&asid).unwrap().record_budget);
    (move || endpoints.upgrade().is_none(), move || records.upgrade().is_none())
}
