//! Hardware-table admission follows its owning unit/domain, never a reused ID.
//! Published backing is released only by an explicit quiescent backend boundary.
use alloc::vec::Vec;

use charlotte_lifecycle::resources::{
    Amount,
    Budget,
};

use super::dma::Error;
use crate::memory::{
    LazyLock,
    Mutex,
    PHYSICAL_FRAME_ALLOCATOR,
    physical::{
        PAddr,
        PhysicalAddress,
    },
};

pub(super) const DOMAIN_PAGES: u64 = 1024;
const UNIT_PAGES: u64 = 2048;
const PAGE: usize = 4096;
mod tests;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    Domain,
    Unit,
}
fn amount(pages: u64) -> Amount {
    Amount {
        pages,
        objects: 0,
    }
}
struct Pool {
    total: Budget,
    clients: Budget,
}
impl Pool {
    fn new(pages: u64) -> Self {
        Self {
            total: Budget::new(amount(pages)),
            clients: Budget::new(amount(pages * 3 / 4)),
        }
    }

    fn reserve(&mut self, pages: u64, scope: Scope) -> Result<(), Error> {
        self.total.reserve(amount(pages)).map_err(|_| Error::MapFailed)?;
        if scope == Scope::Domain && self.clients.reserve(amount(pages)).is_err() {
            self.total.release(amount(pages)).unwrap();
            return Err(Error::MapFailed);
        }
        Ok(())
    }

    fn release(&mut self, pages: u64, scope: Scope) {
        self.total.release(amount(pages)).expect("IOMMU table node charge underflow");
        if scope == Scope::Domain {
            self.clients.release(amount(pages)).expect("IOMMU client charge underflow");
        }
    }
}
static POOL: LazyLock<Mutex<Pool>> = LazyLock::new(|| {
    let pages = (PHYSICAL_FRAME_ALLOCATOR.lock().usable_bytes() / PAGE as u64 / 32).max(1);
    Mutex::new(Pool::new(pages))
});

struct Region {
    base: PAddr,
    pages: usize,
}
#[derive(PartialEq, Eq)]
enum State {
    Unpublished,
    Published,
    Frozen,
    Released,
}

#[must_use]
pub(super) struct Tables {
    scope: Scope,
    pages: u64,
    limit: u64,
    regions: Vec<Region>,
    uncertain: bool,
    state: State,
}
impl Tables {
    pub(super) fn new(scope: Scope) -> Self {
        Self {
            scope,
            pages: 0,
            limit: if scope == Scope::Domain {
                DOMAIN_PAGES
            } else {
                UNIT_PAGES
            },
            regions: Vec::new(),
            uncertain: false,
            state: State::Unpublished,
        }
    }

    /// Prepare private backing before hardware publication. Ordinary rejection
    /// explicitly rolls back its prefix; abandonment never runs physical work.
    pub(super) fn prepare_unpublished<T>(
        scope: Scope,
        prepare: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<(Self, T), Error> {
        let mut tables = Self::new(scope);
        match prepare(&mut tables) {
            Ok(payload) => Ok((tables, payload)),
            Err(error) => {
                let _ = tables.cancel_unpublished();
                Err(error)
            }
        }
    }

    /// This consumes only a private, never hardware-published owner. Rejected
    /// physical release freezes the original charge; Drop cannot retry it.
    pub(super) fn cancel_unpublished(mut self) -> Result<(), Error> {
        if self.state != State::Unpublished {
            return Err(Error::MapFailed);
        }
        self.cancel_private()
    }

    /// Borrowed by the enclosing typed private-domain owner. No published
    /// backing qualifies, and a failed walk remains frozen with its whole charge.
    pub(super) fn cancel_private(&mut self) -> Result<(), Error> {
        self.cancel_private_with(|frame| PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame))
    }

    pub(super) fn cancel_private_with(
        &mut self,
        release: impl FnMut(PAddr) -> Result<(), crate::memory::physical::Error>,
    ) -> Result<(), Error> {
        if self.state != State::Unpublished {
            return Err(Error::MapFailed);
        }
        self.release_with(release)
    }

    pub(super) fn allocate_frame(&mut self) -> Result<PAddr, Error> {
        self.allocate(1, PAGE)
    }

    pub(super) fn allocate(&mut self, pages: usize, alignment: usize) -> Result<PAddr, Error> {
        let frame = self.allocate_with(pages, alignment, |pages, alignment| {
            let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
            if pages == 1 && alignment == PAGE {
                allocator.allocate_frame()
            } else {
                allocator.allocate_contiguous(pages, alignment)
            }
        })?;
        // Serialized constructor fixture: rejection leaves the allocated frame
        // in this exact private ledger, never frees it under the backend guard.
        if self.scope == Scope::Domain
            && self.state == State::Unpublished
            && super::test_reject_private_allocation(self.pages)
        {
            return Err(Error::MapFailed);
        }
        if self.scope == Scope::Unit
            && self.state == State::Unpublished
            && super::unit_initialization::reject_allocation()
        {
            return Err(Error::MapFailed);
        }
        Ok(frame)
    }

    fn allocate_with(
        &mut self,
        pages: usize,
        alignment: usize,
        allocate: impl FnOnce(usize, usize) -> Result<PAddr, crate::memory::physical::Error>,
    ) -> Result<PAddr, Error> {
        self.allocate_prepared_with(
            pages,
            alignment,
            |regions| regions.try_reserve(1).map_err(|_| Error::MapFailed),
            allocate,
        )
    }

    fn allocate_prepared_with(
        &mut self,
        pages: usize,
        alignment: usize,
        prepare_ledger: impl FnOnce(&mut Vec<Region>) -> Result<(), Error>,
        allocate: impl FnOnce(usize, usize) -> Result<PAddr, crate::memory::physical::Error>,
    ) -> Result<PAddr, Error> {
        if !alignment.is_power_of_two() || alignment < PAGE {
            return Err(Error::MapFailed);
        }
        let mut preparation = PreparingRegion::reserve(self, pages)?;
        if prepare_ledger(&mut preparation.tables.regions).is_err() {
            preparation.cancel_unpublished()?;
            return Err(Error::MapFailed);
        }
        match allocate(pages, alignment) {
            Ok(frame) => preparation.frame = Some(frame),
            Err(_) => {
                preparation.cancel_unpublished()?;
                return Err(Error::MapFailed);
            }
        }
        let frame = preparation.frame.unwrap();
        unsafe {
            core::ptr::write_bytes(frame.into_hhdm_mut::<u8>(), 0, pages * PAGE);
        }
        // Disarm before ownership transfer, retaining capacity if interrupted.
        preparation.tables.uncertain = true;
        preparation.finished = true;
        let base = preparation.frame.take().unwrap();
        preparation.tables.regions.push(Region {
            base,
            pages,
        });
        preparation.tables.uncertain = false;
        Ok(base)
    }

    /// Call before the first hardware-visible root/descriptor/base publication.
    pub(super) fn publish(&mut self) {
        assert!(self.state == State::Unpublished && !self.uncertain);
        self.state = State::Published;
    }

    pub(super) fn is_published(&self) -> bool {
        self.state == State::Published && !self.uncertain
    }

    pub(super) fn is_released(&self) -> bool {
        self.state == State::Released
    }

    /// Backend must first detach authority and confirm all configuration/TLB
    /// maintenance/drains. Shared unit backing has no production release path.
    pub(super) fn release(&mut self) -> Result<(), Error> {
        self.release_with(|frame| PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame))
    }

    fn release_with(
        &mut self,
        mut release: impl FnMut(PAddr) -> Result<(), crate::memory::physical::Error>,
    ) -> Result<(), Error> {
        if self.state == State::Released {
            return Ok(());
        }
        if self.state == State::Frozen {
            return Err(Error::MapFailed);
        }
        self.state = State::Frozen; // Never retry a partial physical walk.
        if self.uncertain {
            return Err(Error::MapFailed);
        }
        for region in &self.regions {
            for index in 0..region.pages {
                if release(region.base + index * PAGE).is_err() {
                    crate::logln!(
                        "[IOMMU admission] rejected table release; whole charge retained"
                    );
                    return Err(Error::MapFailed);
                }
            }
        }
        POOL.lock().release(self.pages, self.scope);
        self.pages = 0;
        self.state = State::Released;
        // Ordinary explicit completion releases metadata too. Fallback must
        // retain this ledger instead of implicitly entering the heap allocator.
        drop(core::mem::take(&mut self.regions));
        Ok(())
    }

    pub(super) fn pages(&self) -> u64 {
        self.pages
    }

    pub(super) fn set_limit(&mut self, pages: u64) {
        assert!(
            pages >= self.pages
                && pages
                    <= if self.scope == Scope::Domain {
                        DOMAIN_PAGES
                    } else {
                        UNIT_PAGES
                    }
        );
        self.limit = pages;
    }
}
impl Drop for Tables {
    fn drop(&mut self) {
        // Preserve ledger allocation as well as backing/admission. Vec field
        // destruction would otherwise enter the heap allocator under unknown
        // outer guards. Explicit successful release already empties this field.
        core::mem::forget(core::mem::take(&mut self.regions));
    }
}

struct PreparingRegion<'a> {
    tables: &'a mut Tables,
    pages: usize,
    frame: Option<PAddr>,
    finished: bool,
}
impl<'a> PreparingRegion<'a> {
    fn reserve(tables: &'a mut Tables, pages: usize) -> Result<Self, Error> {
        if pages == 0
            || tables.uncertain
            || !matches!(tables.state, State::Unpublished | State::Published)
            || pages as u64 > tables.limit.saturating_sub(tables.pages)
        {
            return Err(Error::MapFailed);
        }
        POOL.lock().reserve(pages as u64, tables.scope)?;
        tables.pages += pages as u64;
        Ok(Self {
            tables,
            pages,
            frame: None,
            finished: false,
        })
    }

    fn cancel_unpublished(mut self) -> Result<(), Error> {
        self.rollback_with(|frame| PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame))
    }

    fn rollback_with(
        &mut self,
        mut release: impl FnMut(PAddr) -> Result<(), crate::memory::physical::Error>,
    ) -> Result<(), Error> {
        assert!(!self.finished && !self.tables.uncertain);
        // Disarm before invoking adapters; interruption/rejection is terminal.
        self.finished = true;
        self.tables.uncertain = true;
        if let Some(base) = self.frame.take() {
            for index in 0..self.pages {
                release(base + index * PAGE).map_err(|_| Error::MapFailed)?;
            }
        }
        POOL.lock().release(self.pages as u64, self.tables.scope);
        self.tables.pages -= self.pages as u64;
        self.tables.uncertain = false;
        Ok(())
    }
}
impl Drop for PreparingRegion<'_> {
    fn drop(&mut self) {
        if !self.finished {
            // Even reservation-only abandonment consumes original admission.
            // The exclusive parent borrow is the complete containing owner.
            self.tables.uncertain = true;
        }
        // Scalar provisional backing is quarantined; no allocator/pool/table
        // guard, physical callback or logger may be entered from this fallback.
    }
}

pub(super) fn used() -> (u64, u64) {
    let pool = POOL.lock();
    (pool.total.used().pages, pool.clients.used().pages)
}
pub(super) struct ClientPressure(Amount);
impl ClientPressure {
    pub(super) fn new() -> Self {
        let mut pool = POOL.lock();
        let old = pool.clients.limit();
        let used = pool.clients.used();
        pool.clients.set_limit(used).unwrap();
        Self(old)
    }
}
impl Drop for ClientPressure {
    fn drop(&mut self) {
        POOL.lock().clients.set_limit(self.0).unwrap();
    }
}
pub(super) fn test_admission() {
    tests::run();
}

pub(super) fn test_drop_under_guards(action: impl FnOnce()) {
    tests::drop_under_guards(action);
}
