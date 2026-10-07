use alloc::{
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};
use std::{
    alloc::{
        GlobalAlloc,
        Layout,
        System,
    },
    cell::Cell,
};

use super::*;

std::thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct TracedAllocator;
#[global_allocator]
static ALLOCATOR: TracedAllocator = TracedAllocator;

fn note_allocation() {
    if TRACK.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    }
}

// This host-only wrapper records current-test-thread allocations. Delegating
// identical pointer/layout operations preserves System's allocator contract.
unsafe impl GlobalAlloc for TracedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        note_allocation();
        unsafe { System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

fn without_allocation(f: impl FnOnce()) {
    struct Trace;
    impl Drop for Trace {
        fn drop(&mut self) {
            TRACK.with(|track| track.set(false));
        }
    }
    ALLOCATIONS.with(|count| count.set(0));
    TRACK.with(|track| track.set(true));
    let trace = Trace;
    f();
    drop(trace);
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}

struct Tracked(usize, Arc<AtomicUsize>);
impl Drop for Tracked {
    fn drop(&mut self) {
        self.1.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn empty_preparation_rejects_without_publishing_payload() {
    assert!(matches!(
        PreparedEntry::<Tracked>::try_new_with(|node| {
            assert!(node.value.is_none() && node.next.is_none());
            Err(AllocError)
        }),
        Err(AllocError)
    ));
    let prepared = PreparedEntry::<Tracked>::try_new().unwrap();
    without_allocation(|| drop(prepared));
}

#[test]
fn staging_filtering_requeue_and_release_do_not_allocate() {
    let drops = Arc::new(AtomicUsize::new(0));
    let prepared: Vec<_> = (0..1024)
        .map(|id| (PreparedEntry::try_new().unwrap(), Tracked(id, drops.clone())))
        .collect();
    without_allocation(|| {
        let mut queue = RetirementList::new();
        for (storage, value) in prepared {
            queue.push(storage.publish(value));
        }
        let mut detached = core::mem::take(&mut queue);
        detached.reverse();
        let mut deferred = RetirementList::new();
        let mut expected = 0;
        while let Some(entry) = detached.pop() {
            assert_eq!(entry.value().0, expected);
            expected += 1;
            if entry.value().0 % 2 == 0 {
                deferred.push(entry);
            } else {
                entry.release();
            }
        }
        assert_eq!(drops.load(Ordering::Relaxed), 512);
        deferred.reverse();
        while let Some(entry) = deferred.pop() {
            queue.push(entry);
        }
        queue.reverse();
        expected = 0;
        while let Some(entry) = queue.pop() {
            assert_eq!(entry.value().0, expected);
            expected += 2;
            entry.release();
        }
        assert!(queue.is_empty());
    });
    assert_eq!(drops.load(Ordering::Relaxed), 1024);
}

#[test]
fn abandoned_entry_and_list_do_not_destroy_payloads() {
    let drops = Arc::new(AtomicUsize::new(0));
    let entry = PreparedEntry::try_new().unwrap().publish(Tracked(0, drops.clone()));
    let mut list = RetirementList::new();
    list.push(PreparedEntry::try_new().unwrap().publish(Tracked(1, drops.clone())));
    without_allocation(|| {
        drop(entry);
        drop(list);
    });
    assert_eq!(drops.load(Ordering::Relaxed), 0);
}

#[test]
fn independent_heads_preserve_node_identity_and_owned_payloads() {
    let drops = Arc::new(AtomicUsize::new(0));
    let prepared = PreparedEntry::try_new().unwrap();
    let identity = &*prepared.0 as *const Node<Tracked>;
    let mut heads = [RetirementList::new(), RetirementList::new()];
    heads[1].push(prepared.publish(Tracked(7, drops.clone())));
    assert!(heads[0].is_empty());
    assert_eq!(heads[1].iter().next().unwrap().0, 7);
    let entry = heads[1].pop().unwrap();
    assert_eq!(&**entry.0.as_ref().unwrap() as *const Node<Tracked>, identity);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    entry.release();
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}
