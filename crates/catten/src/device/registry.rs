//! Device registry storage uses the same admitted owning nodes as retirement.
//! Lookup/relink/detach never allocate or destroy a node; release is explicit.
use core::{
    fmt,
    mem::ManuallyDrop,
    ops::Index,
};

use super::*;
use crate::klib::collections::retirement_list::{
    PreparedEntry,
    RetiredEntry,
    RetirementList,
};

pub(super) struct Map<K, V>(RetirementList<(K, V)>);
impl<K: Ord + Copy, V> Map<K, V> {
    pub(super) const fn new() -> Self {
        Self(RetirementList::new())
    }

    pub(super) fn get(&self, key: &K) -> Option<&V> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub(super) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub(super) fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.0.iter().map(|(k, v)| (k, v))
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &V> {
        self.0.iter().map(|(_, v)| v)
    }

    pub(super) fn first_key_value(&self) -> Option<(&K, &V)> {
        self.iter().next()
    }

    pub(super) fn insert(&mut self, entry: PreparedEntry<(K, V)>, key: K, value: V) {
        assert!(!self.contains_key(&key), "admitted device key replaced");
        let entry = entry.publish((key, value));
        self.0.insert_before(entry, |(other, _)| *other > key);
    }

    pub(super) fn take(&mut self, key: &K) -> Option<RetiredEntry<(K, V)>> {
        self.0.take_first(|(k, _)| k == key)
    }
}
impl<K: Ord + Copy, V> Default for Map<K, V> {
    fn default() -> Self {
        Self::new()
    }
}
impl<K: Ord + Copy, V> Index<&K> for Map<K, V> {
    type Output = V;

    fn index(&self, key: &K) -> &V {
        self.get(key).expect("device registry key absent")
    }
}
impl<K: Ord + Copy + fmt::Debug, V: fmt::Debug> fmt::Debug for Map<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

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
