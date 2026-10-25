//! Mapping record/snapshot preparation precedes publication and pin mutation.
use super::*;

pub(super) mod tests;

type Storage = PreparedEntry<(AddressSpaceId, MemoryMappingRecord)>;
pub(super) type RetiredMapping = RetiredEntry<(AddressSpaceId, MemoryMappingRecord)>;

struct Resources {
    node: Option<Storage>,
    snapshot: Option<snapshot_storage::Preparing>,
}

/// Partial and published preparation abandonment retains every allocation.
#[must_use]
pub(super) struct Preparing(ManuallyDrop<Resources>);
impl Preparing {
    pub(super) fn new(
        handle: super::super::AddressSpaceHandle,
        pages: usize,
    ) -> Result<Self, MemoryObjectError> {
        Self::with(handle, pages, |_| Ok(()))
    }

    pub(super) fn with(
        handle: super::super::AddressSpaceHandle,
        pages: usize,
        mut before: impl FnMut(u8) -> Result<(), MemoryObjectError>,
    ) -> Result<Self, MemoryObjectError> {
        let mut owner = Self(ManuallyDrop::new(Resources {
            node: None,
            snapshot: None,
        }));
        let result = (|| {
            before(0)?;
            owner.0.node = Some(Storage::try_new().map_err(|_| MemoryObjectError::ResourceLimit)?);
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
    pub(super) fn publish(
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
        MappingRetirementPin::acquire(registry, object)
    }

    pub(super) fn frames(&self) -> &[PAddr] {
        self.0.snapshot.as_ref().unwrap().frames()
    }

    /// Ordinary completion/rejection only, after local memory/table guards.
    pub(super) fn finish(mut self) {
        drop(self.0.node.take());
        if let Some(snapshot) = self.0.snapshot.take() {
            snapshot.finish();
        }
    }
}

/// Only confirmed cleanup may consume a detached node. Keep its exact peer
/// lease through actual node deallocation, then finish that original root last.
pub(super) fn release(node: RetiredMapping) {
    let mut peer = None;
    node.release_with(|(_, mapping)| peer = mapping.cleanup_lease);
    if let Some(peer) = peer {
        peer.release().expect("retiring mapping lost its exact root lease");
    }
}
