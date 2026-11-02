//! Trace actual buffer deallocation while the original production charge lives.
use core::sync::atomic::{
    AtomicU64,
    AtomicUsize,
    Ordering,
};
use std::alloc::{
    GlobalAlloc,
    System,
};

use super::*;
use crate::klib::admission;
struct Tracer;
static WATCH: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static AT_FREE: AtomicU64 = AtomicU64::new(0);
#[global_allocator]
static ALLOCATOR: Tracer = Tracer;
unsafe impl GlobalAlloc for Tracer {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if pointer as usize == WATCH.load(Ordering::Acquire) {
            AT_FREE.store(used().0[0], Ordering::Release);
            FREES.fetch_add(1, Ordering::AcqRel);
        }
        unsafe { System.dealloc(pointer, layout) }
    }
}
fn used() -> ([u64; 2], [u64; 2]) {
    admission::used::<MemoryCapabilityNodes, 2>()
}
fn watch(frames: &[PAddr]) {
    FREES.store(0, Ordering::Release);
    WATCH.store(frames.as_ptr() as usize, Ordering::Release);
}
fn freed(bytes: u64) {
    WATCH.store(0, Ordering::Release);
    assert_eq!(FREES.load(Ordering::Acquire), 1);
    assert_eq!(AT_FREE.load(Ordering::Acquire), bytes);
}
fn allocate(frames: &mut Vec<PAddr>, pages: usize) -> Result<(), MemoryObjectError> {
    frames.try_reserve_exact(pages).map_err(|_| MemoryObjectError::FrameAllocFailed)
}
#[test]
fn popped_identities_retain_capacity_and_refund_follows_actual_free() {
    let before = used();
    let mut vector = FrameVector::try_new_with(false, 8, |frames, pages| {
        assert_eq!(used().0, [before.0[0] + 64, before.0[1]]);
        allocate(frames, pages)
    })
    .unwrap();
    for frame in 0..8 {
        vector.push(frame);
    }
    for frame in (0..8).rev() {
        assert_eq!(vector.pop(), Some(frame));
    }
    assert_eq!(vector.capacity(), 8);
    assert_eq!(used().1, [before.1[0] + 64, before.1[1]]);
    watch(&vector.0.frames);
    vector.finish();
    freed(before.0[0] + 64);
    assert_eq!(used(), before);
}
#[test]
fn allocated_constructor_rejection_frees_before_refund() {
    let before = used();
    assert!(matches!(
        FrameVector::try_new_with(false, 8, |frames, pages| {
            allocate(frames, pages)?;
            watch(frames);
            Err(MemoryObjectError::FrameAllocFailed)
        }),
        Err(MemoryObjectError::FrameAllocFailed)
    ));
    freed(before.0[0] + 64);
    assert_eq!(used(), before);
}
#[test]
fn invalid_size_and_inexact_capacity_reject_without_retained_charge() {
    let before = used();
    for pages in [0, MAX_MEMORY_OBJECT_PAGES + 1, usize::MAX] {
        assert!(matches!(
            FrameVector::try_new_with(false, pages, |_, _| panic!("bound reached allocator")),
            Err(MemoryObjectError::ResourceLimit)
        ));
    }
    assert!(matches!(
        FrameVector::try_new_with(false, 8, |frames, _| allocate(frames, 7)),
        Err(MemoryObjectError::FrameAllocFailed)
    ));
    assert_eq!(used(), before);
}
#[test]
fn byte_quota_precedes_allocator_and_trusted_reserve_is_separate() {
    let before = used();
    let mut paid = FrameVector::try_new(false, 1).unwrap();
    let ceiling = crate::resource_policy::POLICY.memory_capability_nodes.ordinary[0];
    let pressure = Charge::reserve(false, [ceiling - used().1[0], 0]).unwrap();
    assert!(matches!(
        FrameVector::try_new_with(false, 1, |_, _| panic!("quota reached allocator")),
        Err(MemoryObjectError::ResourceLimit)
    ));
    paid.push(42);
    assert_eq!(paid.pop(), Some(42));
    let trusted = FrameVector::try_new(true, 1).unwrap();
    assert_eq!(used().1[0], ceiling);
    trusted.finish();
    drop(pressure);
    paid.finish();
    assert_eq!(used(), before);
}
#[test]
fn abandonment_is_inert_before_and_under_metadata_pool_lock() {
    let before = used();
    let first = FrameVector::try_new(false, 1).unwrap();
    watch(&first.0.frames);
    drop(first);
    WATCH.store(0, Ordering::Release);
    assert_eq!(FREES.load(Ordering::Acquire), 0);
    let second = FrameVector::try_new(false, 1).unwrap();
    watch(&second.0.frames);
    admission::with_locked::<MemoryCapabilityNodes, 2, _>(|| drop(second));
    WATCH.store(0, Ordering::Release);
    assert_eq!(FREES.load(Ordering::Acquire), 0);
    assert_eq!(used(), ([before.0[0] + 16, before.0[1]], [before.1[0] + 16, before.1[1]]));
}
