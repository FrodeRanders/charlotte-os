//! Fixed, pre-admitted physical-address catalog. The containing backing owner
//! supplies physical completion; an empty catalog is not that proof.
use alloc::vec::Vec;
use core::{
    alloc::Layout,
    mem::ManuallyDrop,
    ops::Deref,
};

use super::{
    MAX_MEMORY_OBJECT_PAGES,
    MemoryObjectError,
    PAddr,
};
use crate::klib::admission::{
    MemoryCapabilityNodes,
    Reservation,
};

type Charge = Reservation<MemoryCapabilityNodes, 2>;
#[derive(Debug, Default)]
struct Storage {
    frames: Vec<PAddr>,
    // Buffer free precedes refund on ordinary constructor rejection.
    charge: Option<Charge>,
}
/// Complete address storage and original metadata charge. Drop retains both.
#[must_use]
#[derive(Debug, Default)]
pub(in crate::memory::object) struct FrameVector(ManuallyDrop<Storage>);
impl FrameVector {
    pub(super) fn try_new(platform: bool, pages: usize) -> Result<Self, MemoryObjectError> {
        Self::try_new_with(platform, pages, |frames, pages| {
            frames.try_reserve_exact(pages).map_err(|_| MemoryObjectError::FrameAllocFailed)
        })
    }

    pub(super) fn try_new_with(
        platform: bool,
        pages: usize,
        allocate: impl FnOnce(&mut Vec<PAddr>, usize) -> Result<(), MemoryObjectError>,
    ) -> Result<Self, MemoryObjectError> {
        if pages == 0 || pages > MAX_MEMORY_OBJECT_PAGES {
            return Err(MemoryObjectError::ResourceLimit);
        }
        let bytes = u64::try_from(
            Layout::array::<PAddr>(pages).map_err(|_| MemoryObjectError::ResourceLimit)?.size(),
        )
        .map_err(|_| MemoryObjectError::ResourceLimit)?;
        let mut storage = Storage {
            frames: Vec::new(),
            charge: Some(
                Charge::reserve(platform, [bytes, 0])
                    .map_err(|_| MemoryObjectError::ResourceLimit)?,
            ),
        };
        allocate(&mut storage.frames, pages)?;
        if storage.frames.capacity() != pages || !storage.frames.is_empty() {
            return Err(MemoryObjectError::FrameAllocFailed);
        }
        Ok(Self(ManuallyDrop::new(storage)))
    }

    pub(super) fn push(&mut self, frame: PAddr) {
        assert!(self.0.frames.len() < self.0.frames.capacity(), "frame catalog was not admitted");
        self.0.frames.push(frame);
    }

    pub(in crate::memory::object) fn as_slice(&self) -> &[PAddr] {
        &self.0.frames
    }

    pub(super) fn pop(&mut self) -> Option<PAddr> {
        self.0.frames.pop()
    }

    /// Only confirmed physical completion (or an unused empty preparation)
    /// permits disposal. Popping every identity never proves release by itself.
    pub(super) fn finish(self) {
        assert!(self.0.frames.is_empty(), "live frame identities remain");
        let Storage {
            frames,
            charge,
        } = ManuallyDrop::into_inner(self.0);
        drop(frames);
        drop(charge);
    }

    #[cfg(any(test, feature = "kernel_tests"))]
    pub(super) fn capacity(&self) -> usize {
        self.0.frames.capacity()
    }
}
impl Deref for FrameVector {
    type Target = [PAddr];

    fn deref(&self) -> &[PAddr] {
        self.as_slice()
    }
}
impl<'a> IntoIterator for &'a FrameVector {
    type IntoIter = core::slice::Iter<'a, PAddr>;
    type Item = &'a PAddr;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
#[cfg(test)]
#[path = "frame_vector/host_tests.rs"]
mod host_tests;
#[cfg(feature = "kernel_tests")]
#[path = "frame_vector/tests.rs"]
mod tests;
#[cfg(feature = "kernel_tests")]
pub(super) use tests::run as test_admission;
