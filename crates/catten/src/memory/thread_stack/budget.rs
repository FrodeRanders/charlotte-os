//! Reserve the complete maximum data footprint before stack preparation.
//! Charges are independent of telemetry and never refunded by default Drop.
use charlotte_lifecycle::resources::{
    Amount,
    Budget,
};

use crate::memory::{
    LazyLock,
    Mutex,
    PHYSICAL_FRAME_ALLOCATOR,
};

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

    fn reserve(&mut self, pages: u64, platform: bool) -> Option<()> {
        self.total.reserve(amount(pages)).ok()?;
        if !platform && self.ordinary.reserve(amount(pages)).is_err() {
            self.total.release(amount(pages)).unwrap();
            return None;
        }
        Some(())
    }

    fn release(&mut self, pages: u64, platform: bool) {
        self.total.release(amount(pages)).expect("stack node charge underflow");
        if !platform {
            self.ordinary.release(amount(pages)).expect("stack ordinary charge underflow");
        }
    }
}

static POOL: LazyLock<Mutex<Pool>> = LazyLock::new(|| {
    let pages = (PHYSICAL_FRAME_ALLOCATOR.lock().usable_bytes() / 4096 / 8).max(1);
    Mutex::new(Pool::new(pages))
});

#[derive(Debug)]
#[must_use]
pub(super) struct Reservation {
    pages: u64,
    platform: bool,
}
impl Reservation {
    pub(super) fn reserve(pages: usize, platform: bool) -> Option<Self> {
        let pages = pages as u64;
        POOL.lock().reserve(pages, platform)?;
        Some(Self {
            pages,
            platform,
        })
    }

    /// No backing was allocated, or the whole stack pair's physical cleanup
    /// completed. Consume once; abandonment retains the original node charge.
    pub(super) fn refund(self) {
        POOL.lock().release(self.pages, self.platform);
    }
}

pub(super) fn used() -> (u64, u64) {
    let pool = POOL.lock();
    (pool.total.used().pages, pool.ordinary.used().pages)
}

/// Single-mutator boot adapter; never changes real charges or physical memory.
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

pub(super) fn test_pool() {
    let mut pool = Pool::new(80);
    pool.reserve(60, false).unwrap();
    assert!(pool.reserve(1, false).is_none());
    assert_eq!(pool.total.used().pages, 60);
    pool.reserve(20, true).unwrap();
    assert!(pool.reserve(1, true).is_none());
    pool.release(60, false);
    pool.release(20, true);
    assert_eq!(pool.total.used().pages, 0);
    assert_eq!(pool.ordinary.used().pages, 0);
}
