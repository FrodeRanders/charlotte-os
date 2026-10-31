use std::{
    alloc::{
        GlobalAlloc,
        Layout,
        System,
    },
    cell::Cell,
};

use super::*;

std::thread_local! { static HEAP_CALLS: Cell<Option<(usize, usize)>> = const { Cell::new(None) }; }
struct TracedAllocator;
#[global_allocator]
static ALLOCATOR: TracedAllocator = TracedAllocator;

fn note_heap(allocation: bool) {
    let _ = HEAP_CALLS.try_with(|calls| {
        if let Some((allocations, frees)) = calls.get() {
            calls.set(Some((
                allocations + usize::from(allocation),
                frees + usize::from(!allocation),
            )));
        }
    });
}

unsafe impl GlobalAlloc for TracedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_heap(true);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note_heap(true);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        note_heap(true);
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        note_heap(false);
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[test]
fn full_plan_construction_removal_transfer_and_abandonment_do_not_use_heap() {
    HEAP_CALLS.with(|calls| calls.set(Some((0, 0))));
    for _ in 0..256 {
        let mut phases = RetainedArray::<_, 11>::from_options(core::array::from_fn(Some));
        for expected in 0..11 {
            assert_eq!(phases.first_mut().copied(), Some(expected));
            assert_eq!(phases.remove(0), expected);
        }
        assert!(phases.is_empty());
        assert_eq!(phases.first_mut(), None);
        let mut devices = RetainedArray::<_, 3>::from([1, 2, 3]);
        let mut transferred = core::mem::take(&mut devices);
        assert!(devices.is_empty());
        assert_eq!(transferred.swap_remove(1), 2);
        assert_eq!(transferred[0], 1);
        assert_eq!(transferred[1], 3);
        assert_eq!(transferred.swap_remove(1), 3);
        assert_eq!(transferred.swap_remove(0), 1);
        assert!(transferred.is_empty());
    }
    assert_eq!(HEAP_CALLS.with(|calls| calls.replace(None)), Some((0, 0)));
}

#[test]
fn absent_groups_compact_without_reordering_dependencies() {
    for mask in 0..(1 << 11) {
        let mut phases = RetainedArray::<_, 11>::from_options(core::array::from_fn(|index| {
            (mask & (1 << index) != 0).then_some(index)
        }));
        let expected = (0..11).filter(|index| mask & (1 << index) != 0);
        for phase in expected {
            assert_eq!(phases.remove(0), phase);
        }
        assert!(phases.is_empty());
    }
}

struct Payload<'a>(&'a Cell<usize>, Box<[u8; 32]>);
impl Drop for Payload<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn abandoned_parent_retains_nested_payloads_without_callbacks_or_deallocation() {
    let drops = Cell::new(0);
    let payload = Payload(&drops, Box::new([42; 32]));
    let phases = RetainedArray::<_, 11>::from([RetainedArray::<_, 1>::from([payload])]);
    HEAP_CALLS.with(|calls| calls.set(Some((0, 0))));
    drop(phases);
    assert_eq!(HEAP_CALLS.with(|calls| calls.replace(None)), Some((0, 0)));
    assert_eq!(drops.get(), 0);
}

#[test]
fn explicit_removal_transfers_one_owner_and_never_destroys_its_neighbors() {
    let drops = Cell::new(0);
    let mut owners = RetainedArray::<_, 3>::from([
        Payload(&drops, Box::new([1; 32])),
        Payload(&drops, Box::new([2; 32])),
        Payload(&drops, Box::new([3; 32])),
    ]);
    let removed = owners.remove(1);
    assert_eq!(drops.get(), 0);
    assert_eq!(removed.1[0], 2);
    assert_eq!(owners[0].1[0], 1);
    assert_eq!(owners[1].1[0], 3);
    drop(removed);
    assert_eq!(drops.get(), 1);
    let mut transferred = core::mem::take(&mut owners);
    assert!(owners.is_empty());
    drop(transferred.swap_remove(0));
    assert_eq!(drops.get(), 2);
    drop(transferred.swap_remove(0));
    assert_eq!(drops.get(), 3);
}

#[test]
fn zero_capacity_and_empty_plans_have_no_payload_to_destroy() {
    let mut empty = RetainedArray::<usize, 0>::from([]);
    assert_eq!(empty.len(), 0);
    assert!(empty.is_empty());
    assert_eq!(empty.first_mut(), None);
}
