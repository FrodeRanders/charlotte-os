//! Admission for user heap and loader/runtime backing, in independent pools.
//! An account is embedded in the owning address space: no per-page ledger
//! allocation, reusable-ASID lookup or early refund at logical retirement.

use charlotte_lifecycle::resources::{
    Amount,
    Budget,
};

use super::{
    LazyLock,
    Mutex,
    PHYSICAL_FRAME_ALLOCATOR,
};

pub(crate) const IMAGE_DOMAIN_PAGES: u64 = 16_384; // 64 MiB, including runtime pages.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Heap,
    Image,
}

impl Kind {
    fn pool(self) -> &'static Mutex<Pool> {
        match self {
            Self::Heap => &HEAP_POOL,
            Self::Image => &IMAGE_POOL,
        }
    }

    const fn domain_pages(self) -> u64 {
        match self {
            Self::Heap => (charlotte_launch::HEAP_VA_LIMIT / 4096) as u64,
            Self::Image => IMAGE_DOMAIN_PAGES,
        }
    }
}

#[derive(Debug)]
struct Pool {
    total: Budget,
    ordinary: Budget,
}

fn amount(pages: u64) -> Amount {
    Amount {
        pages,
        objects: 0,
    }
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
        self.total.release(amount(pages)).expect("user backing node charge underflow");
        if !platform {
            self.ordinary.release(amount(pages)).expect("user backing ordinary charge underflow");
        }
    }
}

// Heap, image/runtime and memory-object backing each have a quarter-RAM pool.
// Other physical consumers remain uncharged; this isn't a global RAM ledger.
fn new_pool() -> Mutex<Pool> {
    let pages = (PHYSICAL_FRAME_ALLOCATOR.lock().usable_bytes() / 4096 / 4).max(1);
    Mutex::new(Pool::new(pages))
}
static HEAP_POOL: LazyLock<Mutex<Pool>> = LazyLock::new(new_pool);
static IMAGE_POOL: LazyLock<Mutex<Pool>> = LazyLock::new(new_pool);

#[derive(Debug)]
pub(crate) struct Account {
    kind: Kind,
    pages: u64,
    limit: u64,
    platform: bool,
    retired: bool,
}

impl Account {
    pub(crate) const fn new(kind: Kind) -> Self {
        Self {
            kind,
            pages: 0,
            limit: kind.domain_pages(),
            platform: false,
            retired: false,
        }
    }

    /// Caller retains the address-space table guard through mapping/commit.
    pub(crate) fn reserve(&self) -> Result<PageCharge, ()> {
        if self.retired || self.pages >= self.limit {
            return Err(());
        }
        self.kind.pool().lock().reserve(self.platform)?;
        Ok(PageCharge {
            kind: self.kind,
            platform: self.platform,
            active: true,
        })
    }

    pub(crate) fn commit(&mut self, mut charge: PageCharge) {
        assert!(!self.retired && self.pages < self.limit);
        assert_eq!(self.platform, charge.platform);
        assert_eq!(self.kind, charge.kind);
        self.pages += 1;
        charge.active = false;
    }

    pub(crate) fn accepting(&self) -> bool {
        !self.retired
    }

    pub(crate) fn mark_platform(&mut self) {
        assert!(!self.retired);
        if !self.platform {
            if self.pages != 0 {
                self.kind.pool().lock().ordinary.release(amount(self.pages)).unwrap();
            }
            self.platform = true;
        }
    }

    pub(crate) fn retire(&mut self) {
        self.retired = true;
    }

    /// Kernel policy/testing only; virtual capacity remains independent.
    pub(crate) fn set_limit(&mut self, pages: u64) -> Result<(), ()> {
        if self.retired || pages > self.kind.domain_pages() || pages < self.pages {
            return Err(());
        }
        self.limit = pages;
        Ok(())
    }

    pub(crate) fn pages(&self) -> u64 {
        self.pages
    }
}

impl Drop for Account {
    fn drop(&mut self) {
        // AddressSpace::drop frees its frames before Rust drops this field.
        if self.pages != 0 {
            self.kind.pool().lock().release(self.pages, self.platform);
        }
    }
}

/// Owns the node reservation until a mapped frame joins its address space.
/// Allocator, frame-tracking or mapping failure refunds only this reservation.
#[must_use]
pub(crate) struct PageCharge {
    kind: Kind,
    platform: bool,
    active: bool,
}

impl Drop for PageCharge {
    fn drop(&mut self) {
        if self.active {
            self.kind.pool().lock().release(1, self.platform);
        }
    }
}

pub(crate) fn test_pool() {
    let mut pool = Pool::new(8);
    for _ in 0..6 {
        pool.reserve(false).unwrap();
    }
    assert!(pool.reserve(false).is_err());
    assert_eq!(pool.total.used().pages, 6);
    pool.reserve(true).unwrap();
    pool.reserve(true).unwrap();
    assert!(pool.reserve(true).is_err());
    pool.release(1, false);
    pool.reserve(false).unwrap();
    pool.release(6, false);
    pool.release(2, true);
    assert_eq!(pool.total.used().pages, 0);
    assert_eq!(pool.ordinary.used().pages, 0);

    for kind in [Kind::Heap, Kind::Image] {
        let before = kind.pool().lock().total.used();
        let ordinary_before = kind.pool().lock().ordinary.used();
        let mut account = Account::new(kind);
        account.set_limit(1).unwrap();
        drop(account.reserve().unwrap());
        assert_eq!(kind.pool().lock().total.used(), before);
        account.commit(account.reserve().unwrap());
        assert!(account.reserve().is_err());
        assert!(account.set_limit(0).is_err());
        account.mark_platform();
        account.retire();
        assert!(account.reserve().is_err());
        assert_eq!(account.pages(), 1);
        drop(account);
        assert_eq!(kind.pool().lock().total.used(), before);
        assert_eq!(kind.pool().lock().ordinary.used(), ordinary_before);
    }
}

pub(crate) fn test_used_pages(kind: Kind) -> u64 {
    kind.pool().lock().total.used().pages
}
