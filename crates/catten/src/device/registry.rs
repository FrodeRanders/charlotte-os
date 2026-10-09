//! Device registry storage uses the same admitted owning nodes as retirement.
//! Lookup/relink/detach never allocate or destroy a node; release is explicit.
use core::mem::ManuallyDrop;

use super::*;
pub(super) use crate::klib::collections::retirement_list::AdmittedMap as Map;
use crate::klib::collections::retirement_list::{
    PreparedEntry,
    RetiredEntry,
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
    pub(super) fn try_new() -> Result<Self, DeviceError> {
        // Both allocations precede local lifecycle/device/backend holds. A
        // second-node rejection destroys only empty first-node storage here.
        if tests::reject(1) {
            return Err(DeviceError::ResourceLimit);
        }
        let namespace = PreparedEntry::try_new().map_err(|_| DeviceError::ResourceLimit)?;
        tests::boundary(false);
        if tests::reject(2) {
            tests::boundary(true);
            drop(namespace);
            return Err(DeviceError::ResourceLimit);
        }
        let capability = PreparedEntry::try_new().map_err(|_| DeviceError::ResourceLimit)?;
        tests::boundary(false);
        Ok(Self(ManuallyDrop::new(Storage {
            namespace: Some(namespace),
            capability: Some(capability),
        })))
    }

    pub(super) fn publish(
        &mut self,
        devices: &mut Map<AddressSpaceId, AsDeviceCaps>,
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
        tests::boundary(true);
        drop(ManuallyDrop::into_inner(self.0));
    }
}

pub(super) fn release<K, V>(entry: RetiredEntry<(K, V)>) {
    tests::boundary(true);
    entry.release();
}
pub(super) mod tests;
