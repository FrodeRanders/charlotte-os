//! Shared pin-free vector preparation; copy and DMA keep separate admission
//! predicates, pin publication and completion proofs.
use super::*;

#[must_use]
pub(super) struct Preparing(ManuallyDrop<Vec<PAddr>>);
impl Preparing {
    pub(super) fn new(pages: usize) -> Result<Self, MemoryObjectError> {
        Self::with(pages, |_| Ok(()))
    }

    pub(super) fn with(
        pages: usize,
        mut before: impl FnMut(u8) -> Result<(), MemoryObjectError>,
    ) -> Result<Self, MemoryObjectError> {
        let mut owner = Self(ManuallyDrop::new(Vec::new()));
        let result = (|| {
            before(0)?;
            owner.0.try_reserve_exact(pages).map_err(|_| MemoryObjectError::FrameAllocFailed)?;
            before(1)
        })();
        if let Err(error) = result {
            owner.finish();
            return Err(error);
        }
        Ok(owner)
    }

    /// Caller revalidated its source under memory serialization. No allocation
    /// follows this capacity check; the typed publisher owns pin mutation.
    pub(super) fn fill_and_take(&mut self, frames: &[PAddr]) -> ManuallyDrop<Vec<PAddr>> {
        assert!(self.0.is_empty() && self.0.capacity() >= frames.len());
        self.0.extend_from_slice(frames);
        ManuallyDrop::new(core::mem::take(&mut *self.0))
    }

    pub(super) fn finish(mut self) {
        drop(core::mem::take(&mut *self.0));
    }
}
