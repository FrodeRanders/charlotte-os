//! Mapping record/snapshot preparation precedes publication and pin mutation.
use super::*;

mod admission;
#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory::object) mod tests;

type Storage = PreparedEntry<(AddressSpaceId, MemoryMappingRecord), admission::Charge>;
pub(in crate::memory::object) type Mappings =
    AdmittedMap<AddressSpaceId, MemoryMappingRecord, admission::Charge>;
pub(in crate::memory::object) type RetiredMapping =
    RetiredEntry<(AddressSpaceId, MemoryMappingRecord), admission::Charge>;

struct Resources {
    node: Option<Storage>,
    snapshot: Option<snapshot_storage::Preparing>,
}

/// Partial and published preparation abandonment retains every allocation.
#[must_use]
pub(in crate::memory::object) struct Preparing(ManuallyDrop<Resources>);
impl Preparing {
    pub(in crate::memory::object) fn new(
        handle: crate::memory::AddressSpaceHandle,
        pages: usize,
    ) -> Result<Self, MemoryObjectError> {
        Self::with(handle, pages, |_| Ok(()))
    }

    pub(in crate::memory::object) fn with(
        handle: crate::memory::AddressSpaceHandle,
        pages: usize,
        mut before: impl FnMut(u8) -> Result<(), MemoryObjectError>,
    ) -> Result<Self, MemoryObjectError> {
        // Capture node funding and resource class against the caller's exact
        // generation. The separately admitted snapshot has its own backing class.
        let (captured, storage) =
            crate::memory::capture_address_space_resource_sponsor(handle.id());
        if captured != Some(handle) {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        let mut owner = Self(ManuallyDrop::new(Resources {
            node: None,
            snapshot: None,
        }));
        let result = (|| {
            before(0)?;
            owner.0.node = Some(admission::prepare(storage)?);
            before(1)?;
            owner.0.snapshot = Some(snapshot_storage::Preparing::new(handle, pages)?);
            before(2)
        })();
        if let Err(error) = result {
            owner.finish();
            return Err(error);
        }
        Ok(owner)
    }

    /// Caller revalidated the captured object/root and holds memory serialization.
    /// Fill preallocated snapshot storage and relink before taking the same pin.
    pub(in crate::memory::object) fn publish(
        &mut self,
        registry: &mut MemoryObjectRegistry,
        object: MemoryObjectId,
        asid: AddressSpaceId,
        mapping: MemoryMappingState,
    ) -> MappingRetirementPin {
        let entry = registry.objects.get_mut(&object).expect("validated mapping object missing");
        self.0.snapshot.as_mut().unwrap().fill(&entry.frames);
        entry.mappings.insert(
            self.0.node.take().expect("prepared mapping record missing"),
            asid,
            MemoryMappingRecord {
                state: mapping,
                cleanup_lease: None,
            },
        );
        MappingRetirementPin::acquire(registry, object).with_mapping(mapping)
    }

    pub(in crate::memory::object) fn frames(&self) -> &[PAddr] {
        self.0.snapshot.as_ref().unwrap().frames()
    }

    /// Ordinary completion/rejection only, after local memory/table guards.
    pub(in crate::memory::object) fn finish(mut self) {
        drop(self.0.node.take());
        if let Some(snapshot) = self.0.snapshot.take() {
            snapshot.finish();
        }
    }
}

/// Only confirmed cleanup may consume a detached node. Keep its exact peer
/// lease through actual node deallocation, then finish that original root last.
pub(in crate::memory::object) fn release(node: RetiredMapping) {
    let mut peer = None;
    node.release_with(|(_, mapping)| peer = mapping.cleanup_lease);
    if let Some(peer) = peer {
        peer.release().expect("retiring mapping lost its exact root lease");
    }
}
