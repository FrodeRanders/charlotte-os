//! Fresh runtime kernel tables are shared and retained for the kernel lifetime.
//! A charge follows preparation, then becomes permanent at link publication.
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
    budget: Budget,
    quarantined: u64,
}

// Independent of private roots and data backing. Bootloader-inherited tables
// predate this preparation owner; this pool admits fresh runtime allocations.
static POOL: LazyLock<Mutex<Pool>> = LazyLock::new(|| {
    let pages = (PHYSICAL_FRAME_ALLOCATOR.lock().usable_bytes() / 4096 / 64).max(1);
    Mutex::new(Pool {
        budget: Budget::new(amount(pages)),
        quarantined: 0,
    })
});

#[must_use]
pub(super) struct Charge {
    retaining: bool,
}

impl Charge {
    pub(super) fn reserve() -> Option<Self> {
        POOL.lock().budget.reserve(amount(1)).ok()?;
        Some(Self {
            retaining: true,
        })
    }

    /// Only an unused reservation or confirmed unpublished physical release
    /// permits refund. No published kernel-table release path currently exists.
    pub(super) fn refund(mut self) {
        self.retaining = false;
        POOL.lock().budget.release(amount(1)).expect("shared table charge underflow");
    }

    pub(super) fn publish(mut self) {
        // Sharing/copying a higher-half link does not mint another charge.
        self.retaining = false;
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        if self.retaining {
            // Rejected release, interruption or abandonment retains admission.
            // Drop never refunds or retries potentially reachable backing.
            POOL.lock().quarantined += 1;
        }
    }
}

pub(super) fn used_pages() -> u64 {
    POOL.lock().budget.used().pages
}

pub(super) fn quarantined_pages() -> u64 {
    POOL.lock().quarantined
}

/// Single-mutator boot fixture. It changes available admission, never clears
/// real charges, changes physical memory or recovers quarantined backing.
pub(super) struct Pressure(Amount);

impl Pressure {
    pub(super) fn new(headroom: u64) -> Self {
        let mut pool = POOL.lock();
        let old = pool.budget.limit();
        let used = pool.budget.used().pages;
        assert!(used + headroom <= old.pages);
        pool.budget.set_limit(amount(used + headroom)).unwrap();
        Self(old)
    }
}

impl Drop for Pressure {
    fn drop(&mut self) {
        POOL.lock().budget.set_limit(self.0).unwrap();
    }
}
