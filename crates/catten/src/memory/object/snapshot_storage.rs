//! Shared admitted snapshot buffers; mapping, copy and DMA keep separate
//! publication and physical completion proofs.
use super::*;

mod admission;
pub(super) mod tests;

pub(super) fn admitted() -> ([u64; 2], [u64; 2]) {
    admission::snapshot()
}
pub(super) fn with_budget_locked<T>(work: impl FnOnce() -> T) -> T {
    admission::with_locked(work)
}

struct Resources {
    frames: Vec<PAddr>,
    charge: Option<admission::Charge>,
}

/// Admission follows this allocation through preparation, publication and
/// abandonment. Empty escrow pins have no buffer allocation or reservation.
#[must_use]
pub(super) struct Snapshot(ManuallyDrop<Resources>);
impl Default for Snapshot {
    fn default() -> Self {
        Self(ManuallyDrop::new(Resources {
            frames: Vec::new(),
            charge: None,
        }))
    }
}
impl core::ops::Deref for Snapshot {
    type Target = [PAddr];

    fn deref(&self) -> &[PAddr] {
        &self.0.frames
    }
}
impl Snapshot {
    pub(super) fn as_slice(&self) -> &[PAddr] {
        self
    }

    pub(super) fn finish(self) {
        self.finish_with(|| {});
    }

    fn finish_with(mut self, after_buffer_free: impl FnOnce()) {
        drop(core::mem::take(&mut self.0.frames));
        after_buffer_free();
        drop(self.0.charge.take());
    }
}

#[must_use]
pub(super) struct Preparing(Snapshot);
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
        before: impl FnMut(u8) -> Result<(), MemoryObjectError>,
    ) -> Result<Self, MemoryObjectError> {
        Self::with_factory(handle, pages, before, |frames, pages| {
            frames.try_reserve_exact(pages).map_err(|_| MemoryObjectError::FrameAllocFailed)
        })
    }

    fn with_factory(
        handle: super::super::AddressSpaceHandle,
        pages: usize,
        mut before: impl FnMut(u8) -> Result<(), MemoryObjectError>,
        allocate: impl FnOnce(&mut Vec<PAddr>, usize) -> Result<(), MemoryObjectError>,
    ) -> Result<Self, MemoryObjectError> {
        let bytes = pages
            .checked_mul(core::mem::size_of::<PAddr>())
            .filter(|bytes| *bytes != 0)
            .ok_or(MemoryObjectError::FrameAllocFailed)?;
        let platform = super::super::budget::platform_identity(handle.id()) == Some(handle);
        let mut owner = Self(Snapshot::default());
        let result = (|| {
            before(0)?;
            owner.0.0.charge = Some(admission::reserve(platform, bytes)?);
            allocate(&mut owner.0.0.frames, pages)?;
            before(1)
        })();
        if let Err(error) = result {
            owner.finish();
            return Err(error);
        }
        Ok(owner)
    }

    pub(super) fn fill(&mut self, frames: &[PAddr]) {
        assert!(self.0.0.frames.is_empty() && self.0.0.frames.capacity() >= frames.len());
        self.0.0.frames.extend_from_slice(frames);
    }

    pub(super) fn frames(&self) -> &[PAddr] {
        &self.0
    }

    pub(super) fn fill_and_take(&mut self, frames: &[PAddr]) -> Snapshot {
        self.fill(frames);
        core::mem::take(&mut self.0)
    }

    pub(super) fn finish(self) {
        self.0.finish();
    }
}
