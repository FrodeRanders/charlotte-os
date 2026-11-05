//! Root-owned frame identities and allocation-lifetime byte admission. Storage
//! is retained on abandonment/failed physical release; empty length is no proof.
use alloc::{
    collections::TryReserveError,
    vec::Vec,
};
use core::{
    alloc::AllocError,
    cell::Cell,
    mem::ManuallyDrop,
};

use super::{
    PAddr,
    kernel_buffers::Sponsor,
    translation::Account,
};
use crate::klib::admission::{
    ByteAllocator,
    ByteSponsor,
    MemoryCapabilityNodes,
};

// Only Binding can change this cell, while exclusively borrowing the complete
// tracker. No allocator/cell clone escapes; allocator growth uses the same
// instance. Binding an unidentified allocation first admits its entire capacity.
#[derive(Clone, Debug)]
struct CapturedSponsor(Cell<Sponsor>);
impl ByteSponsor for CapturedSponsor {
    fn reserve(&self, bytes: u64) -> Result<(), AllocError> {
        self.0.get().reserve(bytes)
    }

    fn release(&self, bytes: u64) {
        self.0.get().release(bytes);
    }
}
type Alloc = ByteAllocator<MemoryCapabilityNodes, CapturedSponsor>;

#[derive(Debug)]
#[must_use]
pub(crate) struct FrameTracker(ManuallyDrop<Vec<PAddr, Alloc>>);
impl FrameTracker {
    pub(crate) fn unidentified() -> Self {
        Self::with(false, Sponsor::unidentified())
    }

    pub(crate) fn new(account: &Account) -> Self {
        Self::with(account.is_platform(), Sponsor::from_account(account))
    }

    fn with(platform: bool, sponsor: Sponsor) -> Self {
        Self(ManuallyDrop::new(Vec::new_in(ByteAllocator::with_sponsor(
            platform,
            CapturedSponsor(Cell::new(sponsor)),
        ))))
    }

    pub(crate) fn prepare(&mut self) -> Result<(), TryReserveError> {
        self.0.try_reserve(1)
    }

    pub(crate) fn push(&mut self, frame: PAddr) {
        assert!(self.0.len() < self.0.capacity(), "user frame tracking was not preflighted");
        self.0.push(frame);
    }

    pub(crate) fn drain(&mut self) -> impl Iterator<Item = PAddr> + '_ {
        self.0.drain(..)
    }

    /// Only the completed owning physical walk authorizes this call. No frame
    /// address is released here and no popped/empty catalog proves completion.
    pub(crate) fn finish(&mut self) {
        assert!(self.0.is_empty());
        let allocator = self.0.allocator().clone();
        let buffer = core::mem::replace(&mut *self.0, Vec::new_in(allocator));
        drop(buffer); // Actual deallocation precedes both byte refunds.
    }

    /// Admission precedes every identity mutation. The exclusive borrow keeps
    /// this precise buffer stable while the root's other accounts bind.
    pub(crate) fn prepare_binding(&mut self, principal: u64) -> Result<Binding<'_>, AllocError> {
        let previous = self.0.allocator().sponsor().0.get().principal();
        if previous.is_some_and(|old| old != principal) {
            return Err(AllocError);
        }
        let sponsor = Sponsor::verified(principal);
        let bytes = if previous.is_none() {
            self.0.capacity().checked_mul(core::mem::size_of::<PAddr>()).ok_or(AllocError)? as u64
        } else {
            0
        };
        if bytes != 0 {
            sponsor.reserve(bytes)?;
        }
        Ok(Binding {
            tracker: self,
            sponsor,
            bytes,
        })
    }

    #[cfg(any(test, feature = "kernel_tests"))]
    pub(crate) fn capacity_bytes(&self) -> u64 {
        (self.0.capacity() * core::mem::size_of::<PAddr>()) as u64
    }
}

/// Unstarted funding for this exact borrowed buffer. Abandonment retains its
/// reservation without locks, allocation or refund under an unknown context.
#[must_use]
pub(crate) struct Binding<'a> {
    tracker: &'a mut FrameTracker,
    sponsor: Sponsor,
    bytes: u64,
}
impl Binding<'_> {
    pub(crate) fn commit(self) {
        self.tracker.0.allocator().sponsor().0.set(self.sponsor);
    }

    pub(crate) fn cancel(self) {
        if self.bytes != 0 {
            self.sponsor.release(self.bytes);
        }
    }
}

#[cfg(test)]
#[path = "frame_tracking/host_tests.rs"]
mod host_tests;
#[cfg(feature = "kernel_tests")]
#[path = "frame_tracking/tests.rs"]
pub(crate) mod tests;
