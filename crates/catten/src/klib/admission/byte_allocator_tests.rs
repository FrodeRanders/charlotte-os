//! Standalone host harness: actual Vec growth/deallocation through production
//! admission. It is separate from other harnesses' global allocator tracers.
use alloc::vec::Vec;
use core::{
    alloc::{
        Allocator,
        Layout,
    },
    sync::atomic::{
        AtomicBool,
        AtomicU64,
        AtomicUsize,
        Ordering,
    },
};
use std::alloc::{
    GlobalAlloc,
    System,
};

use crate::klib::admission::{
    self,
    ByteAllocator,
    ByteSponsor,
    IpcNodes,
    Reservation,
};
type A = ByteAllocator<IpcNodes>;
static TRACK: AtomicBool = AtomicBool::new(false);
static PEAK: AtomicU64 = AtomicU64::new(0);
static WATCH: AtomicUsize = AtomicUsize::new(0);
static AT_FREE: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static SPONSOR_USED: AtomicU64 = AtomicU64::new(0);
static SPONSOR_LIMIT: AtomicU64 = AtomicU64::new(u64::MAX);
static SPONSOR_PEAK: AtomicU64 = AtomicU64::new(0);
static SPONSOR_AT_FREE: AtomicU64 = AtomicU64::new(0);
struct Tracer;
#[global_allocator]
static GLOBAL: Tracer = Tracer;
unsafe impl GlobalAlloc for Tracer {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.load(Ordering::Acquire) {
            PEAK.fetch_max(used().0[0], Ordering::AcqRel);
            SPONSOR_PEAK.fetch_max(SPONSOR_USED.load(Ordering::Acquire), Ordering::AcqRel);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        if WATCH.load(Ordering::Acquire) == pointer as usize {
            AT_FREE.store(used().0[0], Ordering::Release);
            SPONSOR_AT_FREE.store(SPONSOR_USED.load(Ordering::Acquire), Ordering::Release);
            FREES.fetch_add(1, Ordering::AcqRel);
        }
    }
}
fn used() -> ([u64; 2], [u64; 2]) {
    admission::used::<IpcNodes, 2>()
}
fn watch<T>(value: &[T]) {
    FREES.store(0, Ordering::Release);
    WATCH.store(value.as_ptr() as usize, Ordering::Release);
}
#[test]
fn real_growth_reserves_full_overlap_and_actual_free_precedes_refund() {
    let before = used();
    let mut values = Vec::new_in(A::new(false));
    values.try_reserve_exact(2).unwrap();
    values.extend([7u64, 9]);
    assert_eq!(values.capacity(), 2);
    watch(&values);
    PEAK.store(0, Ordering::Release);
    TRACK.store(true, Ordering::Release);
    values.try_reserve_exact(1).unwrap();
    TRACK.store(false, Ordering::Release);
    WATCH.store(0, Ordering::Release);
    assert_eq!(PEAK.load(Ordering::Acquire), before.0[0] + 16 + 24);
    assert_eq!(FREES.load(Ordering::Acquire), 1);
    assert_eq!(AT_FREE.load(Ordering::Acquire), before.0[0] + 16 + 24);
    assert_eq!(values.as_slice(), [7, 9]);
    assert_eq!(values.capacity(), 3);
    assert_eq!(used(), ([before.0[0] + 24, before.0[1]], [before.1[0] + 24, before.1[1]]));
    watch(&values);
    drop(values);
    WATCH.store(0, Ordering::Release);
    assert_eq!(AT_FREE.load(Ordering::Acquire), before.0[0] + 24);
    assert_eq!(FREES.load(Ordering::Acquire), 1);
    assert_eq!(used(), before);
}
#[test]
fn rejected_growth_preserves_original_pointer_capacity_and_prefix() {
    let before = used();
    let mut values = Vec::new_in(A::new(false));
    values.try_reserve_exact(48).unwrap();
    values.resize(48, 0x5au8);
    let pointer = values.as_ptr();
    let limit = crate::resource_policy::POLICY.ipc_nodes.ordinary[0];
    let pressure =
        Reservation::<IpcNodes, 2>::reserve(false, [limit - used().1[0] - 16, 0]).unwrap();
    let full = used();
    assert!(values.try_reserve_exact(8).is_err());
    assert_eq!(used(), full);
    assert_eq!(values.as_ptr(), pointer);
    assert_eq!(values.capacity(), 48);
    assert_eq!(values.as_slice(), [0x5a; 48]);
    drop(pressure);
    drop(values);
    assert_eq!(used(), before);
}
#[test]
fn backend_rejection_refunds_only_unstarted_allocation() {
    let before = used();
    for platform in [false, true] {
        assert!(A::new(platform).test_allocation_rejection(Layout::new::<[u8; 64]>()).is_err());
        assert_eq!(used(), before);
    }
}
#[test]
fn pop_clear_move_and_clone_keep_allocation_class_and_capacity() {
    let before = used();
    let mut values = Vec::new_in(A::new(true));
    values.try_reserve_exact(4).unwrap();
    values.extend([1u64, 2, 3, 4]);
    assert_eq!(values.pop(), Some(4));
    values.clear();
    assert_eq!(values.capacity(), 4);
    assert_eq!(used().0[0], before.0[0] + 32);
    assert_eq!(used().1, before.1);
    values.extend([8, 9]);
    let cloned = values.clone();
    assert!(cloned.allocator().platform());
    assert_eq!(used().1, before.1);
    let moved = values;
    assert!(moved.allocator().platform());
    drop((moved, cloned));
    assert_eq!(used(), before);
}
#[test]
fn zero_sized_storage_and_allocator_clones_reserve_nothing() {
    let before = used();
    let allocator = A::new(true);
    let cloned = allocator.clone();
    let layout = Layout::from_size_align(0, 8).unwrap();
    let block = allocator.allocate(layout).unwrap();
    unsafe { cloned.deallocate(block.cast(), layout) };
    let mut empty = Vec::<(), _>::new_in(allocator);
    empty.try_reserve_exact(100).unwrap();
    drop(empty);
    assert_eq!(used(), before);
}

#[derive(Clone, Copy, Debug)]
struct Sponsor;
impl ByteSponsor for Sponsor {
    fn reserve(&self, bytes: u64) -> Result<(), core::alloc::AllocError> {
        SPONSOR_USED.fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
            used.checked_add(bytes).filter(|next| *next <= SPONSOR_LIMIT.load(Ordering::Acquire))
        }).map(|_| ()).map_err(|_| core::alloc::AllocError)
    }
    fn release(&self, bytes: u64) {
        let old = SPONSOR_USED.fetch_sub(bytes, Ordering::AcqRel);
        assert!(old >= bytes);
    }
}

#[test]
fn captured_sponsor_funds_complete_growth_and_refunds_only_after_actual_free() {
    let before = used();
    assert_eq!(SPONSOR_USED.load(Ordering::Acquire), 0);
    let allocator = ByteAllocator::<IpcNodes, _>::with_sponsor(false, Sponsor);
    let mut values = Vec::<u64, _>::new_in(allocator.clone());
    values.try_reserve_exact(2).unwrap();
    values.extend([7, 9]);
    watch(&values);
    SPONSOR_PEAK.store(0, Ordering::Release);
    TRACK.store(true, Ordering::Release);
    values.try_reserve_exact(1).unwrap();
    TRACK.store(false, Ordering::Release);
    WATCH.store(0, Ordering::Release);
    assert_eq!(SPONSOR_PEAK.load(Ordering::Acquire), 16 + 24);
    assert_eq!(SPONSOR_AT_FREE.load(Ordering::Acquire), 16 + 24);
    assert_eq!(SPONSOR_USED.load(Ordering::Acquire), 24);
    SPONSOR_LIMIT.store(24, Ordering::Release);
    let pointer = values.as_ptr();
    assert!(values.try_reserve_exact(2).is_err());
    assert_eq!(values.as_ptr(), pointer);
    assert_eq!(values.as_slice(), [7, 9]);
    assert_eq!(values.capacity(), 3);
    assert_eq!(SPONSOR_USED.load(Ordering::Acquire), 24);
    SPONSOR_LIMIT.store(u64::MAX, Ordering::Release);
    let full = crate::resource_policy::POLICY.ipc_nodes.ordinary[0];
    let pressure = Reservation::<IpcNodes, 2>::reserve(false, [full-used().1[0], 0]).unwrap();
    assert!(allocator.allocate(Layout::new::<u64>()).is_err());
    assert_eq!(SPONSOR_USED.load(Ordering::Acquire), 24);
    drop(pressure);
    assert!(allocator.test_allocation_rejection(Layout::new::<u64>()).is_err());
    assert_eq!(SPONSOR_USED.load(Ordering::Acquire), 24);
    watch(&values);
    drop(values);
    WATCH.store(0, Ordering::Release);
    assert_eq!(SPONSOR_AT_FREE.load(Ordering::Acquire), 24);
    assert_eq!(SPONSOR_USED.load(Ordering::Acquire), 0);
    assert_eq!(used(), before);
    let zero = Layout::from_size_align(0, 8).unwrap();
    let block = allocator.allocate(zero).unwrap();
    unsafe { allocator.deallocate(block.cast(), zero) };
    assert_eq!(SPONSOR_USED.load(Ordering::Acquire), 0);
}
