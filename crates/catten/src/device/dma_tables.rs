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

    pub(super) fn allocate_frame(&mut self) -> Result<PAddr, Error> {
        self.allocate(1, PAGE)
    }

    pub(super) fn allocate(&mut self, pages: usize, alignment: usize) -> Result<PAddr, Error> {
        self.allocate_with(pages, alignment, |pages, alignment| {
            let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
            if pages == 1 && alignment == PAGE {
                allocator.allocate_frame()
            } else {
                allocator.allocate_contiguous(pages, alignment)
            }
        })
    }

    fn allocate_with(
        &mut self,
        pages: usize,
        alignment: usize,
        allocate: impl FnOnce(usize, usize) -> Result<PAddr, crate::memory::physical::Error>,
    ) -> Result<PAddr, Error> {
        if pages == 0
            || !alignment.is_power_of_two()
            || alignment < PAGE
            || self.uncertain
            || !matches!(self.state, State::Unpublished | State::Published)
            || pages as u64 > self.limit.saturating_sub(self.pages)
        {
            return Err(Error::MapFailed);
        }
        POOL.lock().reserve(pages as u64, self.scope)?;
        self.pages += pages as u64;
        let mut preparation = PreparingRegion {
            tables: self,
            pages,
            frame: None,
            finished: false,
        };
        preparation.tables.regions.try_reserve(1).map_err(|_| Error::MapFailed)?;
        preparation.frame = Some(allocate(pages, alignment).map_err(|_| Error::MapFailed)?);
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
        assert!(self.state == State::Unpublished);
        self.state = State::Published;
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
        if self.state == State::Unpublished {
            let _ = self.release();
        }
        // Published/uncertain/frozen backing and its original charge remain.
    }
}

struct PreparingRegion<'a> {
    tables: &'a mut Tables,
    pages: usize,
    frame: Option<PAddr>,
    finished: bool,
}
impl Drop for PreparingRegion<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Some(base) = self.frame.take() {
            self.tables.uncertain = true;
            for index in 0..self.pages {
                if PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(base + index * PAGE).is_err() {
                    return;
                }
            }
            self.tables.uncertain = false;
        }
        POOL.lock().release(self.pages as u64, self.tables.scope);
        self.tables.pages -= self.pages as u64;
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
