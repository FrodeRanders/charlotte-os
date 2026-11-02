//! Ledger capacity is admitted before allocation. Each buffer retains its
//! original class through replacement, physical uncertainty and actual free.
use alloc::vec::Vec;
use core::{
    alloc::Layout,
    mem::{
        self,
        ManuallyDrop,
    },
};

use super::{
    Error,
    Region,
};
use crate::klib::admission::{
    IommuNodes,
    Reservation,
};

type Charge = Reservation<IommuNodes, 2>;
struct Backing {
    regions: Vec<Region>,
    // Field order frees the buffer before refund on ordinary rejection.
    charge: Option<Charge>,
}
impl Backing {
    fn empty() -> Self {
        Self {
            regions: Vec::new(),
            charge: None,
        }
    }

    fn prepare(
        platform: bool,
        capacity: usize,
        allocate: impl FnOnce(&mut Vec<Region>, usize) -> Result<(), Error>,
    ) -> Result<Self, Error> {
        let bytes =
            u64::try_from(Layout::array::<Region>(capacity).map_err(|_| Error::MapFailed)?.size())
                .map_err(|_| Error::MapFailed)?;
        // Byte-only reservation: the pool's node count describes actual nodes.
        // Full candidate capacity overlaps the complete old buffer and charge.
        let mut candidate = Self {
            regions: Vec::new(),
            charge: Some(Charge::reserve(platform, [bytes, 0]).map_err(|_| Error::MapFailed)?),
        };
        allocate(&mut candidate.regions, capacity)?;
        if candidate.regions.capacity() != capacity {
            return Err(Error::MapFailed);
        }
        Ok(candidate)
    }

    fn finish(self, after_free: impl FnOnce()) {
        let Self {
            regions,
            charge,
        } = self;
        drop(regions);
        after_free();
        drop(charge);
    }
}
/// Complete owning metadata. Implicit field destruction must retain both the
/// addresses and their buffer charge; only confirmed table completion releases.
pub(super) struct Ledger(ManuallyDrop<Backing>);
impl Ledger {
    pub(super) fn new() -> Self {
        Self(ManuallyDrop::new(Backing::empty()))
    }

    pub(super) fn iter(&self) -> core::slice::Iter<'_, Region> {
        self.0.regions.iter()
    }

    pub(super) fn push(&mut self, region: Region) {
        assert!(self.0.regions.len() < self.0.regions.capacity());
        self.0.regions.push(region);
    }

    pub(super) fn prepare(&mut self, platform: bool, limit: u64) -> Result<(), Error> {
        self.prepare_with(
            platform,
            limit,
            |regions, capacity| regions.try_reserve_exact(capacity).map_err(|_| Error::MapFailed),
            || {},
        )
    }

    fn prepare_with(
        &mut self,
        platform: bool,
        limit: u64,
        allocate: impl FnOnce(&mut Vec<Region>, usize) -> Result<(), Error>,
        after_old_free: impl FnOnce(),
    ) -> Result<(), Error> {
        if self.0.regions.len() < self.0.regions.capacity() {
            return Ok(());
        }
        let limit = usize::try_from(limit).map_err(|_| Error::MapFailed)?;
        let capacity = self
            .0
            .regions
            .len()
            .checked_add(1)
            .and_then(|n| n.max(4).checked_next_power_of_two())
            .map(|n| n.min(limit))
            .filter(|n| *n > self.0.regions.len())
            .ok_or(Error::MapFailed)?;
        let candidate = Backing::prepare(platform, capacity, allocate)?;
        // From here, an interruption must retain authoritative addresses and
        // both buffers/charges, rather than run ordinary candidate cleanup.
        let mut candidate = ManuallyDrop::new(candidate);
        candidate.regions.append(&mut self.0.regions);
        mem::swap(&mut *self.0, &mut *candidate);
        let old = ManuallyDrop::into_inner(candidate);
        debug_assert!(old.regions.is_empty());
        old.finish(after_old_free);
        Ok(())
    }

    /// Caller has confirmed complete physical release. No destruction on Drop.
    pub(super) fn release(self) {
        ManuallyDrop::into_inner(self.0).finish(|| {});
    }

    #[cfg(any(test, feature = "kernel_tests"))]
    pub(super) fn capacity(&self) -> usize {
        self.0.regions.capacity()
    }

    #[cfg(any(test, feature = "kernel_tests"))]
    pub(super) fn is_empty(&self) -> bool {
        self.0.regions.is_empty()
    }
}

#[cfg(test)]
#[path = "ledger/host_tests.rs"]
mod host_tests;
#[cfg(feature = "kernel_tests")]
#[path = "ledger/tests.rs"]
mod tests;
#[cfg(feature = "kernel_tests")]
pub(super) use tests::run as test_admission;
