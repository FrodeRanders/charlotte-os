//! Mapping record/snapshot preparation precedes publication and pin mutation.
use super::*;

pub(super) mod tests;

type Storage = PreparedEntry<(AddressSpaceId, MemoryMappingRecord)>;
pub(super) type RetiredMapping = RetiredEntry<(AddressSpaceId, MemoryMappingRecord)>;

pub(super) struct Preparing {
    node: Option<Storage>,
    pub(super) frames: Vec<PAddr>,
}
impl Preparing {
    pub(super) fn new(pages: usize) -> Result<Self, MemoryObjectError> {
        Self::with(pages, |_| Ok(()))
    }

    pub(super) fn with(
        pages: usize,
        mut before: impl FnMut(u8) -> Result<(), MemoryObjectError>,
    ) -> Result<Self, MemoryObjectError> {
        before(0)?;
        let node = Storage::try_new().map_err(|_| MemoryObjectError::ResourceLimit)?;
        before(1)?;
        let mut frames = Vec::new();
        frames.try_reserve_exact(pages).map_err(|_| MemoryObjectError::FrameAllocFailed)?;
        before(2)?;
        Ok(Self {
            node: Some(node),
            frames,
        })
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
        assert!(self.frames.is_empty() && self.frames.capacity() >= entry.frames.len());
        self.frames.extend_from_slice(&entry.frames);
        entry.mappings.insert(
            self.node.take().expect("prepared mapping record missing"),
            asid,
            MemoryMappingRecord {
                state: mapping,
                cleanup_lease: None,
            },
        );
        MappingRetirementPin::acquire(registry, object)
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
