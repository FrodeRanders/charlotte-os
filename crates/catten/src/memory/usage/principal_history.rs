//! Stack and heap history share one boot-lifetime record per signed principal.
//! Each root prepares a candidate before publication. Identity installation
//! links it only for a new principal; an unused candidate stays with the root
//! until confirmed final invalidation permits explicit metadata disposal.
use core::{
    alloc::AllocError,
    mem::ManuallyDrop,
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct History {
    stack_pages: u64,
    heap_bytes: u64,
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

pub(super) mod tests;
