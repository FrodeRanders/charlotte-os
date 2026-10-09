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
    static DEALLOCATIONS: Cell<usize> = const { Cell::new(0) };
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
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            let _ = DEALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        }
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
    let mut entry = heads[1].pop().unwrap();
    assert_eq!(&**entry.0.as_ref().unwrap() as *const Node<Tracked>, identity);
    without_allocation(|| entry.value_mut().0 = 8);
    assert_eq!(entry.value().0, 8);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    entry.release();
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn ordered_relink_lookup_and_detachment_do_no_allocator_work() {
    let mut list = RetirementList::new();
    let entries =
        [4usize, 1, 3, 2].map(|key| PreparedEntry::try_new().unwrap().publish((key, key * 10)));
    DEALLOCATIONS.with(|n| n.set(0));
    without_allocation(|| {
        for entry in entries {
            let key = entry.value().0;
            list.insert_before(entry, |v| v.0 > key);
        }
        for (expected, value) in list.iter_mut().enumerate() {
            assert_eq!(value.0, expected + 1);
            value.1 += 1;
        }
        assert!(list.take_first(|v| v.0 == 5).is_none());
        let middle = list.take_first(|v| v.0 == 3).unwrap();
        assert_eq!(middle.value(), &(3, 31));
        list.insert_before(middle, |v| v.0 > 3);
        let tail = list.take_first(|v| v.0 == 4).unwrap();
        list.insert_before(tail, |_| false);
    });
    assert_eq!(DEALLOCATIONS.with(Cell::get), 0);
    for expected in 1..=4 {
        let entry = list.pop().unwrap();
        assert_eq!(entry.value(), &(expected, expected * 10 + 1));
        entry.release();
    }
    assert!(list.is_empty());
}

#[test]
fn ordered_detached_owner_and_list_abandonment_do_no_allocator_work() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut list = RetirementList::new();
    for key in [2, 1, 3] {
        let entry = PreparedEntry::try_new().unwrap().publish(Tracked(key, drops.clone()));
        list.insert_before(entry, |v| v.0 > key);
    }
    let node = list.take_first(|v| v.0 == 2).unwrap();
    DEALLOCATIONS.with(|n| n.set(0));
    without_allocation(|| {
        drop(node);
        drop(list);
    });
    assert_eq!(DEALLOCATIONS.with(Cell::get), 0);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
}

#[test]
fn admitted_map_keeps_detached_node_identity_without_allocator_work() {
    let mut map = AdmittedMap::new();
    let prepared = [3usize, 1, 2].map(|key| (PreparedEntry::try_new().unwrap(), key));
    DEALLOCATIONS.with(|n| n.set(0));
    let detached = {
        let mut detached = None;
        without_allocation(|| {
            for (node, key) in prepared {
                map.insert(node, key, key * 10);
            }
            assert_eq!(map.first_key_value(), Some((&1, &10)));
            assert_eq!(map.values().copied().sum::<usize>(), 60);
            *map.get_mut(&2).unwrap() += 1;
            assert_eq!(map[&2], 21);
            assert!(!map.contains_key(&4));
            assert!(map.take(&4).is_none());
            detached = map.take(&2);
            assert_eq!(map.iter().map(|(key, _)| *key).sum::<usize>(), 4);
        });
        detached.unwrap()
    };
    assert_eq!(DEALLOCATIONS.with(Cell::get), 0);
    assert_eq!(detached.value(), &(2, 21));
    detached.release();
    for key in [1, 3] {
        map.take(&key).unwrap().release();
    }
    assert!(map.first_key_value().is_none());
}

#[test]
fn admitted_map_abandonment_retains_owning_payloads_without_allocator_work() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut map = AdmittedMap::new();
    map.insert(PreparedEntry::try_new().unwrap(), 1usize, Tracked(1, drops.clone()));
    DEALLOCATIONS.with(|n| n.set(0));
    without_allocation(|| drop(map));
    assert_eq!(DEALLOCATIONS.with(Cell::get), 0);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
}
