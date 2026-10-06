//! Private translation admission follows the owning root, never a reusable ASID.
use charlotte_lifecycle::resources::{
    Amount,
    Budget,
};

use crate::memory::{
    LazyLock,
    Mutex,
    PHYSICAL_FRAME_ALLOCATOR,
};

pub(crate) const DOMAIN_TABLE_PAGES: u64 = 1024; // 4 MiB of translation metadata.

fn amount(pages: u64) -> Amount {
    Amount {
        pages,
        objects: 0,
    }
}

struct Pool {
    total: Budget,
    ordinary: Budget,
}

impl Pool {
    fn new(pages: u64) -> Self {
        Self {
            total: Budget::new(amount(pages)),
            ordinary: Budget::new(amount(pages * 3 / 4)),
        }
    }

    fn reserve(&mut self, platform: bool) -> Result<(), ()> {
        self.total.reserve(amount(1)).map_err(|_| ())?;
        if !platform && self.ordinary.reserve(amount(1)).is_err() {
            self.total.release(amount(1)).unwrap();
            return Err(());
        }
        Ok(())
    }

    fn release(&mut self, pages: u64, platform: bool) {
        self.total.release(amount(pages)).expect("table node charge underflow");
        if !platform {
            self.ordinary.release(amount(pages)).expect("table ordinary charge underflow");
        }
    }
}

// Independent of heap/image/object backing: sparse trees need not track data
// density. Ordinary roots cannot consume the last quarter of this pool.
static POOL: LazyLock<Mutex<Pool>> = LazyLock::new(|| {
    let pages = (PHYSICAL_FRAME_ALLOCATOR.lock().usable_bytes() / 4096 / 16).max(1);
    Mutex::new(Pool::new(pages))
});

#[derive(Debug)]
pub(crate) struct Account {
    pages: u64,
    limit: u64,
    platform: bool,
    retired: bool,
    refundable: bool,
    quarantined_pages: u64,
}

impl Account {
    pub(crate) const fn new() -> Self {
        Self {
            pages: 0,
            limit: DOMAIN_TABLE_PAGES,
            platform: false,
            retired: false,
            // Only the complete physical destructor walk can authorize refund.
            refundable: false,
            quarantined_pages: 0,
        }
    }

    pub(super) fn reserve(&mut self) -> Result<(), ()> {
        if self.retired || self.pages >= self.limit {
            return Err(());
        }
        POOL.lock().reserve(self.platform)?;
        self.pages += 1;
        Ok(())
    }

    pub(super) fn refund_unpublished(&mut self) {
        assert!(self.pages > self.quarantined_pages);
        POOL.lock().release(1, self.platform);
        self.pages -= 1;
    }

    pub(super) fn quarantine_unpublished(&mut self) {
        assert!(self.quarantined_pages < self.pages);
        self.quarantined_pages += 1;
    }

    pub(super) fn refund_quarantined(&mut self) {
        assert!(self.quarantined_pages != 0);
        self.quarantined_pages -= 1;
        self.refund_unpublished();
    }

    pub(crate) fn mark_platform(&mut self) {
        assert!(!self.retired && !self.refundable);
        assert_eq!(self.quarantined_pages, 0, "cannot reclassify quarantined tables");
        if !self.platform {
            POOL.lock().ordinary.release(amount(self.pages)).unwrap();
            self.platform = true;
        }
    }

    pub(crate) fn retire(&mut self) {
        self.retired = true;
    }

    pub(crate) fn confirm_release(&mut self) {
        assert!(self.retired);
        self.refundable = true;
    }

    pub(crate) fn set_limit(&mut self, pages: u64) -> Result<(), ()> {
        if self.retired || pages > DOMAIN_TABLE_PAGES || pages < self.pages {
            return Err(());
        }
        self.limit = pages;
        Ok(())
    }

    pub(crate) fn pages(&self) -> u64 {
        self.pages
    }

    pub(super) fn is_platform(&self) -> bool {
        self.platform
    }
}

impl Drop for Account {
    fn drop(&mut self) {
        if self.refundable {
            POOL.lock().release(self.pages - self.quarantined_pages, self.platform);
        }
    }
}

pub(super) fn test_pool() {
    let mut pool = Pool::new(8);
    for _ in 0..6 {
        pool.reserve(false).unwrap();
    }
    assert!(pool.reserve(false).is_err());
    pool.reserve(true).unwrap();
    pool.reserve(true).unwrap();
    assert!(pool.reserve(true).is_err());
    pool.release(1, false);
    pool.reserve(false).unwrap();
    pool.release(6, false);
    pool.release(2, true);
    assert_eq!(pool.total.used().pages, 0);
    assert_eq!(pool.ordinary.used().pages, 0);
}

pub(crate) fn test_used_pages() -> u64 {
    POOL.lock().total.used().pages
}

pub(crate) fn test_ordinary_pages() -> u64 {
    POOL.lock().ordinary.used().pages
}

/// Single-mutator boot admission adapter: exhaust only ordinary admission,
/// leaving real counters, platform reserve and physical memory intact.
pub(super) struct OrdinaryPressure(Amount);

impl OrdinaryPressure {
    pub(super) fn new() -> Self {
        let mut pool = POOL.lock();
        let old = pool.ordinary.limit();
        let used = pool.ordinary.used();
        pool.ordinary.set_limit(used).unwrap();
        Self(old)
    }
}

impl Drop for OrdinaryPressure {
    fn drop(&mut self) {
        POOL.lock().ordinary.set_limit(self.0).unwrap();
    }
}
