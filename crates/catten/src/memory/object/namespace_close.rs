//! Namespace memory cleanup borrows its closing root and retains mapped peers
//! in admitted mapping nodes. No root/capability/mapping snapshot is allocated.

use super::*;
use crate::memory::{
    AddressSpaceHandle,
    retirement::ClosingAddressSpace,
};

#[must_use]
pub(crate) enum MemoryProgress {
    Pending,
    Complete(NamespaceMemoryClosed),
}

#[must_use]
pub(crate) struct NamespaceMemoryClosed {
    handle: AddressSpaceHandle,
}

impl NamespaceMemoryClosed {
    pub(crate) fn handle(&self) -> AddressSpaceHandle {
        self.handle
    }
}

#[must_use]
pub(crate) struct NamespaceObjectClosed {
    handle: AddressSpaceHandle,
    object: MemoryObjectId,
}

#[must_use]
pub(crate) struct PreparedNamespaceObject<'root> {
    root: &'root ClosingAddressSpace,
    receipt: RetiredObjectMappings,
}

// Until every peer is retained, mappings stay visible in the registry. A
// competing namespace cleanup sees the pin and returns Pending, never sealing
// a root whose mapping has not yet been leased. Drop retains the pin and any
// already installed leases. Ordinary admission rejection explicitly rolls back.
struct PreparingNamespaceObject<'root> {
    root: &'root ClosingAddressSpace,
    pin: MappingRetirementPin,
    owning: bool,
}

impl<'root> PreparingNamespaceObject<'root> {
    fn retain_peers(&self) -> Result<(), MemoryObjectError> {
        let handle = self.root.memory_cleanup_handle();
        let mut cursor = None;
        loop {
            let next = {
                let registry = MEMORY_OBJECTS.lock();
                let object = &registry.objects[&self.pin.object];
                object.mappings.iter().find_map(|(&asid, mapping)| {
                    ((self.owning || asid == handle.id()) && cursor.is_none_or(|old| asid > old))
                        .then_some((asid, mapping.state))
                })
            };
            let Some((asid, mapping)) = next else {
                break;
            };
            cursor = Some(asid);
            if mapping.address_space.id() != asid {
                return Err(MemoryObjectError::AddressSpaceMissing);
            }
            if asid == handle.id() {
                if mapping.address_space != handle {
                    return Err(MemoryObjectError::AddressSpaceMissing);
                }
                continue;
            }
            if asid == super::super::KERNEL_ASID {
                if super::super::current_address_space_handle(asid) != Some(mapping.address_space) {
                    return Err(MemoryObjectError::AddressSpaceMissing);
                }
                continue; // Permanent root; never acquire a user-root lease.
            }
            let lease = self.root.retain_peer(mapping.address_space).map_err(|error| {
                if error == OperationError::Limit {
                    MemoryObjectError::ResourceLimit
                } else {
                    MemoryObjectError::AddressSpaceMissing
                }
            })?;
            let mut registry = MEMORY_OBJECTS.lock();
            let object =
                registry.objects.get_mut(&self.pin.object).expect("preparing object missing");
            let stored = object.mappings.get_mut(&asid).expect("pinned mapping disappeared");
            assert_eq!(stored.state, mapping, "pinned mapping changed before peer publication");
            assert!(stored.cleanup_lease.is_none());
            stored.cleanup_lease = Some(lease);
        }
        Ok(())
    }

    fn cancel(self) {
        let mut cursor = None;
        loop {
            let lease = {
                let mut registry = MEMORY_OBJECTS.lock();
                let object =
                    registry.objects.get_mut(&self.pin.object).expect("preparing object missing");
                object.mappings.iter_mut().find_map(|(&asid, mapping)| {
                    if cursor.is_none_or(|old| asid > old) && mapping.cleanup_lease.is_some() {
                        cursor = Some(asid);
                        mapping.cleanup_lease.take()
                    } else {
                        None
                    }
                })
            };
            let Some(lease) = lease else {
                break;
            };
            lease.release().expect("preparation lost its exact peer lease");
        }
        self.pin.release(None);
    }

    fn publish(self) -> PreparedNamespaceObject<'root> {
        let handle = self.root.memory_cleanup_handle();
        let mut registry = MEMORY_OBJECTS.lock();
        let object = registry.objects.get_mut(&self.pin.object).expect("preparing object missing");
        let mappings = if self.owning {
            object.destroy_when_unpinned = true;
            RetiredMappings::All(core::mem::take(&mut object.mappings))
        } else {
            RetiredMappings::One(handle.id(), object.mappings.remove(&handle.id()).unwrap())
        };
        let receipt = RetiredObjectMappings {
            pin: self.pin,
            pages: object.frames.len(),
            mappings,
            detached: false,
        };
        drop(registry);
        PreparedNamespaceObject {
            root: self.root,
            receipt,
        }
    }
}

impl PreparedNamespaceObject<'_> {
    pub(crate) fn finish(self) -> Result<NamespaceObjectClosed, MemoryObjectError> {
        self.finish_with(
            unmap_pages,
            |asid, base, pages| {
                crate::cpu::isa::memory::tlb::try_inval_range_user(asid, base, pages).is_ok()
            },
            release_scratch,
        )
    }

    fn finish_with(
        self,
        unmap: impl FnMut(AddressSpaceId, VAddr, &[PAddr]) -> Result<(), MemoryObjectError>,
        invalidate: impl FnMut(AddressSpaceId, VAddr, usize) -> bool,
        release: impl FnMut(AddressSpaceId, VAddr, usize) -> Result<(), MemoryObjectError>,
    ) -> Result<NamespaceObjectClosed, MemoryObjectError> {
        let handle = self.root.memory_cleanup_handle();
        let object = self.receipt.pin.object;
        self.receipt.detach_with(unmap).complete_with_scratch(handle.id(), invalidate, release)?;
        Ok(NamespaceObjectClosed {
            handle,
            object,
        })
    }
}

pub(crate) fn close_with(
    root: &ClosingAddressSpace,
    mut finish: impl FnMut(
        PreparedNamespaceObject<'_>,
    ) -> Result<NamespaceObjectClosed, MemoryObjectError>,
) -> Result<MemoryProgress, MemoryObjectError> {
    let handle = root.memory_cleanup_handle();
    loop {
        let preparing = {
            let mut registry = MEMORY_OBJECTS.lock();
            let next = registry.objects.iter().find_map(|(&id, object)| {
                ((object.owner == handle.id() && !object.destroy_when_unpinned)
                    || object.mappings.contains_key(&handle.id()))
                .then_some(id)
            });
            let Some(id) = next else {
                break;
            };
            let object = &registry.objects[&id];
            if object.retirement_pins != 0
                || matches!(object.lend_state, LendState::Revoking)
                || object.owner != handle.id() && (object.dma_pins != 0 || object.copy_pins != 0)
            {
                return Ok(MemoryProgress::Pending);
            }
            let owning = object.owner == handle.id();
            PreparingNamespaceObject {
                root,
                pin: MappingRetirementPin::acquire(&mut registry, id),
                owning,
            }
        };
        if let Err(error) = preparing.retain_peers() {
            preparing.cancel();
            return Err(error);
        }
        let object = preparing.pin.object;
        let closed = finish(preparing.publish())?;
        assert_eq!(closed.handle, handle);
        assert_eq!(closed.object, object);
    }
    let mut registry = MEMORY_OBJECTS.lock();
    // Unmapped borrowed authority can also be protected by another revocation
    // or DMA/copy operation. Do not erase it and later restore a stale borrower
    // through that operation's owned prior state. Destroy-fenced objects cannot
    // resume usable authority; their independent pins retain backing/charges.
    if registry.caps.get(&handle.id()).is_some_and(|caps| {
        caps.caps.values().any(|cap| {
            let Some(object) = registry.objects.get(&cap.object) else {
                return false;
            };
            !object.destroy_when_unpinned
                && (cap.transfer_in_flight
                    || object.retirement_pins != 0
                    || object.dma_pins != 0
                    || object.copy_pins != 0
                    || matches!(object.lend_state, LendState::Revoking))
        })
    }) {
        return Ok(MemoryProgress::Pending);
    }
    for object in registry.objects.values_mut() {
        clear_borrower(object, handle.id());
    }
    if let Some(caps) = registry.caps.remove(&handle.id()) {
        for cap in caps.caps.keys() {
            assert!(crate::capability::remove_for_teardown(
                handle.id(),
                *cap,
                crate::capability::ObjectKind::Memory
            ));
        }
    }
    Ok(MemoryProgress::Complete(NamespaceMemoryClosed {
        handle,
    }))
}

pub(crate) mod tests;
