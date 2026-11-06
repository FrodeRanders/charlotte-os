//! Device registry storage uses the same admitted owning nodes as retirement.
//! Lookup/relink/detach never allocate or destroy a node; release is explicit.
use core::mem::ManuallyDrop;

pub(super) use super::node_budget::{
    Map as DeviceMap,
    Retired as RetiredEntry,
};
use super::{
    node_budget::Prepared as PreparedEntry,
    *,
};

struct Storage {
    namespace: Option<PreparedEntry<(AddressSpaceId, AsDeviceCaps)>>,
    capability: Option<PreparedEntry<(DeviceCap, DeviceObject)>>,
}
/// Empty prepublication storage joins the containing grant's retaining owner.
/// Its implicit field fallback must not deallocate below unknown guards.
#[must_use]
pub(super) struct PreparedStorage(ManuallyDrop<Storage>);
impl PreparedStorage {
    pub(super) fn try_new(
        storage: impl Into<crate::memory::kernel_buffers::CapturedStorage>,
    ) -> Result<Self, DeviceError> {
        let storage = storage.into();
        // Both allocations precede local lifecycle/device/backend holds. A
        // second-node rejection destroys only empty first-node storage here.
        if test_fault!(tests::reject(1)) {
            return Err(DeviceError::ResourceLimit);
        }
        let namespace = node_budget::prepare(storage).map_err(|_| DeviceError::ResourceLimit)?;
        #[cfg(feature = "kernel_tests")]
        tests::boundary(false);
        if test_fault!(tests::reject(2)) {
            #[cfg(feature = "kernel_tests")]
            tests::boundary(true);
            drop(namespace);
            return Err(DeviceError::ResourceLimit);
        }
        let capability = node_budget::prepare(storage).map_err(|_| DeviceError::ResourceLimit)?;
        #[cfg(feature = "kernel_tests")]
        tests::boundary(false);
        Ok(Self(ManuallyDrop::new(Storage {
            namespace: Some(namespace),
            capability: Some(capability),
        })))
    }

    pub(super) fn publish(
        &mut self,
        devices: &mut DeviceMap<AddressSpaceId, AsDeviceCaps>,
        owner: AddressSpaceId,
        cap: DeviceCap,
        object: DeviceObject,
    ) {
        if !devices.contains_key(&owner) {
            devices.insert(self.0.namespace.take().unwrap(), owner, AsDeviceCaps::new());
        }
        devices.get_mut(&owner).unwrap().caps.insert(
            self.0.capability.take().unwrap(),
            cap,
            object,
        );
    }

    pub(super) fn release(self) {
        #[cfg(feature = "kernel_tests")]
        tests::boundary(true);
        drop(ManuallyDrop::into_inner(self.0));
    }
}

pub(super) fn release<K, V>(entry: RetiredEntry<(K, V)>) {
    #[cfg(feature = "kernel_tests")]
    tests::boundary(true);
    entry.release();
}
pub(super) fn release_authority(record: crate::capability::RetiredRecord) {
    #[cfg(feature = "kernel_tests")]
    tests::authority_boundary(true);
    record.release();
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) mod tests;
