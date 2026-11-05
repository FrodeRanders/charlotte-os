//! Actual retaining tracker/allocator, with host identity/counter adapters.
//! Native controls supply real roots, history records and physical outcomes.
use core::sync::atomic::{
    AtomicU64,
    AtomicUsize,
    Ordering,
};
use std::alloc::{
    GlobalAlloc,
    Layout,
    System,
};

use super::*;
use crate::{
    klib::admission,
    memory::kernel_buffers::{
        test_limit,
        test_used,
        total_used,
    },
};

static WATCH: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static AT_FREE: AtomicU64 = AtomicU64::new(0);
struct Tracer;
#[global_allocator]
static GLOBAL: Tracer = Tracer;
unsafe impl GlobalAlloc for Tracer {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        if pointer as usize == WATCH.load(Ordering::Acquire) {
            AT_FREE.store(total_used(), Ordering::Release);
            FREES.fetch_add(1, Ordering::AcqRel);
        }
    }
}
fn pool() -> ([u64; 2], [u64; 2]) {
    admission::used::<MemoryCapabilityNodes, 2>()
}
fn watch(tracker: &FrameTracker) {
    FREES.store(0, Ordering::Release);
    WATCH.store(tracker.0.as_ptr() as usize, Ordering::Release);
}

#[test]
fn growth_rejection_preserves_owned_prefix_and_free_precedes_original_refund() {
    let before = pool();
    let total_before = total_used();
    let mut tracker = FrameTracker::with(false, Sponsor::verified(1));
    tracker.prepare().unwrap();
    for n in 1..=4 {
        tracker.push(n);
    }
    let pointer = tracker.0.as_ptr();
    test_limit(1, 32);
    assert!(tracker.prepare().is_err());
    assert_eq!(tracker.0.as_slice(), [1, 2, 3, 4]);
    assert_eq!(tracker.0.as_ptr(), pointer);
    assert_eq!(test_used(1), 32);
    test_limit(1, 96);
    watch(&tracker);
    tracker.prepare().unwrap();
    WATCH.store(0, Ordering::Release);
    assert_eq!(FREES.load(Ordering::Acquire), 1);
    assert_eq!(AT_FREE.load(Ordering::Acquire), total_before + 32 + 64);
    assert_eq!(test_used(1), 64);
    assert_eq!(tracker.0.as_slice(), [1, 2, 3, 4]);
    assert_eq!(tracker.drain().count(), 4);
    assert_eq!(test_used(1), 64, "empty length refunded capacity");
    watch(&tracker);
    tracker.finish();
    WATCH.store(0, Ordering::Release);
    assert_eq!(AT_FREE.load(Ordering::Acquire), total_before + 64);
    assert_eq!(test_used(1), 0);
    assert_eq!(pool(), before);
}

#[test]
fn late_binding_reserves_full_capacity_then_commits_or_cancels_once() {
    let before = pool();
    let mut tracker = FrameTracker::unidentified();
    tracker.prepare().unwrap();
    tracker.push(7);
    let charged = pool();
    test_limit(2, 31);
    assert!(tracker.prepare_binding(2).is_err());
    assert_eq!(test_used(2), 0);
    test_limit(2, 32);
    tracker.prepare_binding(2).unwrap().cancel();
    assert_eq!(test_used(2), 0);
    tracker.prepare_binding(2).unwrap().commit();
    assert_eq!(test_used(2), 32);
    assert!(!tracker.0.allocator().platform());
    assert_eq!(tracker.0.as_slice(), [7]);
    assert_eq!(pool(), charged);
    tracker.prepare_binding(2).unwrap().commit(); // Already funded, no duplicate.
    assert_eq!(test_used(2), 32);
    assert!(tracker.prepare_binding(1).is_err());
    assert_eq!(tracker.drain().count(), 1);
    tracker.finish();
    assert_eq!(test_used(2), 0);
    assert_eq!(pool(), before);
}

#[test]
fn abandonment_keeps_real_allocation_and_both_original_charges() {
    let mut tracker = FrameTracker::with(false, Sponsor::verified(3));
    tracker.prepare().unwrap();
    let charged = pool();
    watch(&tracker);
    drop(tracker);
    WATCH.store(0, Ordering::Release);
    assert_eq!(FREES.load(Ordering::Acquire), 0, "unconfirmed abandonment freed backing");
    assert_eq!(pool(), charged);
    assert_eq!(test_used(3), 32);
}

#[test]
fn unfinished_binding_and_tracker_abandonment_are_inert_under_pool_guard() {
    let mut tracker = FrameTracker::unidentified();
    tracker.prepare().unwrap();
    let charged = pool();
    let binding = tracker.prepare_binding(4).unwrap();
    admission::with_locked::<MemoryCapabilityNodes, 2, _>(|| drop(binding));
    assert_eq!(test_used(4), 32);
    admission::with_locked::<MemoryCapabilityNodes, 2, _>(|| drop(tracker));
    assert_eq!(pool(), charged);
    assert_eq!(test_used(4), 32);
}
