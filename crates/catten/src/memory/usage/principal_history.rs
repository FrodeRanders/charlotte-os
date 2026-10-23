//! History and aggregate backing share one boot-lifetime record per signed principal.
//! Each root prepares a candidate before publication. Identity installation
//! links it only for a new principal; an unused candidate stays with the root
//! until confirmed final invalidation permits explicit metadata disposal.
use core::{
    alloc::AllocError,
    mem::ManuallyDrop,
};

use charlotte_lifecycle::resources::{
    Amount,
    Budget,
    VectorBudget,
};

use crate::{
    klib::collections::retirement_list::{
        AdmittedMap,
        PreparedEntry,
    },
    memory::{
        LazyLock,
        Mutex,
    },
};

/// Shared by every occupancy of one authenticated artifact principal.
/// Image/runtime, demand heap and memory-object pages consume this same limit.
pub(in crate::memory) const MAX_PRINCIPAL_PAGES: u64 = 16_384;
pub(in crate::memory) const MAX_PRINCIPAL_OBJECTS: u64 = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct History {
    stack_pages: u64,
    heap_bytes: u64,
    backing: Budget,
    thread_metadata: VectorBudget<2>,
}

impl Default for History {
    fn default() -> Self {
        Self {
            stack_pages: 0,
            heap_bytes: 0,
            thread_metadata: VectorBudget::new([1024 * 1024, 128]),
            backing: Budget::new(Amount {
                pages: MAX_PRINCIPAL_PAGES,
                objects: MAX_PRINCIPAL_OBJECTS,
            }),
        }
    }
}

type Table = AdmittedMap<u64, History>;
static HISTORY: LazyLock<Mutex<Table>> = LazyLock::new(|| Mutex::new(Table::new()));

/// Inert fallback retains even a candidate that has never been published.
#[must_use]
pub(in crate::memory) struct PreparingHistory(ManuallyDrop<Option<PreparedEntry<(u64, History)>>>);

impl PreparingHistory {
    pub(in crate::memory) fn try_new() -> Result<Self, AllocError> {
        Ok(Self(ManuallyDrop::new(Some(PreparedEntry::try_new()?))))
    }

    /// Called after exact-root identity checks, before authority becomes visible.
    /// Keeping a losing candidate avoids deallocation beneath the caller's guards.
    pub(in crate::memory) fn publish(&mut self, principal: u64) {
        self.publish_into(&mut HISTORY.lock(), principal);
    }

    fn publish_into(&mut self, history: &mut Table, principal: u64) {
        assert_ne!(principal, 0);
        if !history.contains_key(&principal) {
            history.insert(
                self.0.take().expect("root lost history preparation"),
                principal,
                History::default(),
            );
        }
    }

    /// Ordinary cancellation or confirmed root metadata disposal, after guards.
    pub(in crate::memory) fn cancel_unpublished(mut self) {
        drop(self.0.take());
    }
}

/// Linear share of a principal's aggregate reservation. Generation owners keep
/// it through transfer/retirement; retained physical backing retains this charge.
#[derive(Debug)]
#[must_use]
pub(crate) struct BackingCharge {
    principal: u64,
    amount: Amount,
}

impl BackingCharge {
    pub(in crate::memory) fn try_new(principal: u64, amount: Amount) -> Result<Self, ()> {
        let mut history = HISTORY.lock();
        history.get_mut(&principal).ok_or(())?.backing.reserve(amount).map_err(|_| ())?;
        Ok(Self {
            principal,
            amount,
        })
    }

    pub(in crate::memory) fn reserve(&self, amount: Amount) -> Result<Self, ()> {
        Self::try_new(self.principal, amount)
    }

    pub(in crate::memory) fn principal(&self) -> u64 {
        self.principal
    }

    pub(in crate::memory) fn amount(&self) -> Amount {
        self.amount
    }

    pub(in crate::memory) fn split(&mut self, amount: Amount) -> Self {
        self.amount.pages =
            self.amount.pages.checked_sub(amount.pages).expect("principal split pages");
        self.amount.objects =
            self.amount.objects.checked_sub(amount.objects).expect("principal split objects");
        Self {
            principal: self.principal,
            amount,
        }
    }

    pub(in crate::memory) fn absorb(&mut self, mut other: Self) {
        assert_eq!(self.principal, other.principal);
        self.amount.pages =
            self.amount.pages.checked_add(other.amount.pages).expect("principal merge pages");
        self.amount.objects =
            self.amount.objects.checked_add(other.amount.objects).expect("principal merge objects");
        other.amount = Amount::default();
    }

    /// Caller has confirmed release (or never allocated backing). Logical exit,
    /// namespace removal and platform promotion cannot supply that proof.
    pub(in crate::memory) fn release(&mut self, amount: Amount) {
        let released = self.split(amount);
        drop(released);
    }
}

impl Drop for BackingCharge {
    fn drop(&mut self) {
        if self.amount != Amount::default() {
            HISTORY
                .lock()
                .get_mut(&self.principal)
                .expect("principal sponsor missing")
                .backing
                .release(self.amount)
                .expect("principal backing underflow");
        }
    }
}

/// Trusted signed-loader entry, after artifact verification and before even
/// initial private-root backing. Shared admitted storage remains boot-lifetime.
pub(crate) fn prepare_backing(principal: u64) -> Result<BackingCharge, AllocError> {
    if principal == 0 {
        return Err(AllocError);
    }
    let mut preparation = PreparingHistory::try_new()?;
    preparation.publish(principal);
    preparation.cancel_unpublished();
    Ok(BackingCharge::try_new(principal, Amount::default()).expect("prepared principal missing"))
}

pub(crate) fn backing_used(principal: u64) -> Amount {
    HISTORY.lock().get(&principal).map_or(Amount::default(), |entry| entry.backing.used())
}

pub(super) fn remember_stack(principal: u64, pages: u64) {
    let mut history = HISTORY.lock();
    let entry = history.get_mut(&principal).expect("signed principal lost stack history");
    entry.stack_pages = entry.stack_pages.max(pages);
}

pub(super) fn remember_heap(principal: u64, bytes: u64) {
    let mut history = HISTORY.lock();
    let entry = history.get_mut(&principal).expect("signed principal lost heap history");
    entry.heap_bytes = entry.heap_bytes.max(bytes);
}

pub(super) fn stack_high_water(principal: u64) -> u64 {
    HISTORY.lock().get(&principal).map_or(0, |entry| entry.stack_pages)
}

pub(super) fn heap_peak(principal: u64) -> u64 {
    HISTORY.lock().get(&principal).map_or(0, |entry| entry.heap_bytes)
}

pub(super) fn with_locked<R>(work: impl FnOnce() -> R) -> R {
    let _history = HISTORY.lock();
    work()
}

pub(super) fn available() -> bool {
    HISTORY.try_lock().is_some()
}

pub(in crate::memory) mod admission_tests;
mod structural_tests;
pub(super) mod tests;
mod thread_metadata_tests;

/// Separate dimensions for requested heap bytes and complete thread bundles.
/// This charge belongs to the allocation, independent of root-slot lifetime.
#[derive(Debug)]
pub(crate) struct ThreadMetadataCharge {
    principal: u64,
    amount: [u64; 2],
}

impl ThreadMetadataCharge {
    pub(in crate::memory) fn try_new(principal: u64, amount: [u64; 2]) -> Result<Self, ()> {
        HISTORY
            .lock()
            .get_mut(&principal)
            .ok_or(())?
            .thread_metadata
            .reserve(amount)
            .map_err(|_| ())?;
        Ok(Self {
            principal,
            amount,
        })
    }
}

impl Drop for ThreadMetadataCharge {
    fn drop(&mut self) {
        HISTORY
            .lock()
            .get_mut(&self.principal)
            .expect("thread metadata principal missing")
            .thread_metadata
            .release(self.amount)
            .expect("principal thread metadata underflow");
    }
}
