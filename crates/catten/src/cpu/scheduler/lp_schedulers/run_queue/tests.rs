use std::{
    alloc::{
        GlobalAlloc,
        Layout,
        System,
    },
    cell::Cell,
    collections::VecDeque,
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

// Only the current test thread is counted. The exact System operation and
// layout are preserved; this wrapper changes no allocation lifetime.
unsafe impl GlobalAlloc for TracedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            ALLOCATIONS.with(|count| count.set(count.get() + 1));
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            ALLOCATIONS.with(|count| count.set(count.get() + 1));
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            ALLOCATIONS.with(|count| count.set(count.get() + 1));
        }
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            DEALLOCATIONS.with(|count| count.set(count.get() + 1));
        }
        unsafe { System.dealloc(pointer, layout) }
    }
}

fn without_heap(f: impl FnOnce()) {
    struct Tracking;
    impl Drop for Tracking {
        fn drop(&mut self) {
            TRACK.with(|track| track.set(false));
        }
    }
    ALLOCATIONS.with(|count| count.set(0));
    DEALLOCATIONS.with(|count| count.set(0));
    TRACK.with(|track| track.set(true));
    let guard = Tracking;
    f();
    drop(guard);
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert_eq!(DEALLOCATIONS.with(Cell::get), 0);
}

struct Store {
    entries: [(u64, QueueLink); 8],
}
impl Store {
    fn new() -> Self {
        Self {
            entries: core::array::from_fn(|_| (1, QueueLink::new())),
        }
    }
}
impl Entries for Store {
    fn link(&self, handle: ThreadHandle) -> Option<&QueueLink> {
        self.entries
            .get(handle.tid)
            .filter(|(generation, _)| *generation == handle.generation)
            .map(|(_, link)| link)
    }

    fn link_mut(&mut self, handle: ThreadHandle) -> Option<&mut QueueLink> {
        self.entries
            .get_mut(handle.tid)
            .filter(|(generation, _)| *generation == handle.generation)
            .map(|(_, link)| link)
    }
}
fn key(tid: usize) -> ThreadHandle {
    ThreadHandle {
        tid,
        generation: 1,
    }
}
fn assert_order(queue: &RunQueue, store: &Store, expected: &[usize]) {
    assert_eq!(queue.len(), expected.len());
    let mut actual = queue.iter(store);
    for tid in expected {
        assert_eq!(actual.next(), Some(Ok(key(*tid))));
    }
    assert_eq!(actual.next(), None);
}

#[test]
fn fifo_middle_removal_and_transfer_preserve_exact_membership_without_heap() {
    let mut store = Store::new();
    without_heap(|| {
        let mut source = RunQueue::new();
        let mut destination = RunQueue::new();
        for tid in 0..4 {
            source.push_back(&mut store, key(tid)).unwrap();
        }
        for tid in 4..6 {
            destination.push_back(&mut store, key(tid)).unwrap();
        }
        source.remove(&mut store, key(2)).unwrap();
        assert_order(&source, &store, &[0, 1, 3]);
        source.transfer_to(&mut destination, &mut store, key(1)).unwrap();
        assert_order(&source, &store, &[0, 3]);
        assert_order(&destination, &store, &[4, 5, 1]);
        source.remove(&mut store, key(0)).unwrap();
        source.remove(&mut store, key(3)).unwrap();
        assert_order(&source, &store, &[]);
        for tid in [4, 5, 1] {
            assert_eq!(destination.pop_front(&mut store).unwrap(), Some(key(tid)));
        }
        assert_eq!(destination.pop_front(&mut store).unwrap(), None);
        for (_, link) in &store.entries {
            assert!(!link.is_linked());
        }
    });
}

#[test]
fn rejected_destination_and_stale_or_foreign_requests_leave_source_unchanged() {
    let mut store = Store::new();
    let mut source = RunQueue::new();
    let mut destination = RunQueue::new();
    source.push_back(&mut store, key(0)).unwrap();
    source.push_back(&mut store, key(1)).unwrap();
    destination.push_back(&mut store, key(2)).unwrap();
    without_heap(|| {
        assert_eq!(destination.push_back(&mut store, key(0)), Err(Error::AlreadyLinked));
        assert_eq!(destination.remove(&mut store, key(0)), Err(Error::WrongQueue));
        assert_eq!(
            source.transfer_to(
                &mut destination,
                &mut store,
                ThreadHandle {
                    tid: 0,
                    generation: 2
                }
            ),
            Err(Error::InvalidThread)
        );
        // A corrupt destination must reject before unlinking the exact source.
        let tail = destination.tail;
        destination.tail = Some(ThreadHandle {
            tid: 2,
            generation: 2,
        });
        assert_eq!(
            source.transfer_to(&mut destination, &mut store, key(0)),
            Err(Error::InvalidThread)
        );
        assert_order(&source, &store, &[0, 1]);
        destination.tail = tail;
        assert_order(&destination, &store, &[2]);
        source.transfer_to(&mut destination, &mut store, key(0)).unwrap();
        assert_order(&source, &store, &[1]);
        assert_order(&destination, &store, &[2, 0]);
    });
}

#[test]
fn missing_generation_cannot_adopt_or_clear_a_successor_link() {
    let mut store = Store::new();
    let mut queue = RunQueue::new();
    queue.push_back(&mut store, key(0)).unwrap();
    let original_owner = store.entries[0].1.owner;
    // Deliberate illegal owner replacement: no production removal may do
    // this while linked. Qualification must fail without touching its fields.
    store.entries[0].0 = 2;
    let successor = ThreadHandle {
        tid: 0,
        generation: 2,
    };
    without_heap(|| {
        assert_eq!(queue.pop_front(&mut store), Err(Error::InvalidThread));
        assert_eq!(queue.remove(&mut store, successor), Err(Error::InvalidThread));
        assert_eq!(queue.push_back(&mut store, successor), Err(Error::InvalidThread));
        assert_eq!(store.entries[0].1.owner, original_owner);
        assert_eq!(queue.len(), 1);
    });
}

#[test]
fn abandoned_queue_keeps_original_membership_and_new_queue_cannot_adopt_it() {
    let mut store = Store::new();
    let mut queue = RunQueue::new();
    queue.push_back(&mut store, key(0)).unwrap();
    let owner = store.entries[0].1.owner;
    without_heap(|| {
        drop(queue);
        assert!(store.entries[0].1.is_linked());
        let mut replacement = RunQueue::new();
        assert_ne!(Some(replacement.identity), owner);
        assert_eq!(replacement.push_back(&mut store, key(0)), Err(Error::AlreadyLinked));
        assert_eq!(replacement.remove(&mut store, key(0)), Err(Error::WrongQueue));
        assert_eq!(store.entries[0].1.owner, owner);
    });
}

#[test]
fn corrupt_cycle_is_bounded_and_reported_without_clearing_owner_links() {
    let mut store = Store::new();
    let mut queue = RunQueue::new();
    for tid in 0..4 {
        queue.push_back(&mut store, key(tid)).unwrap();
    }
    store.entries[1].1.next = Some(key(1));
    without_heap(|| {
        let mut iter = queue.iter(&store);
        assert_eq!(iter.next(), Some(Ok(key(0))));
        assert_eq!(iter.next(), Some(Ok(key(1))));
        assert_eq!(iter.next(), Some(Err(Error::Corrupt)));
        assert_eq!(iter.next(), None);
        assert_eq!(queue.len(), 4);
        assert!(store.entries.iter().take(4).all(|(_, link)| link.is_linked()));
    });
}

#[test]
fn mixed_operations_match_independent_fifo_models_without_heap() {
    let mut store = Store::new();
    let mut queues = core::array::from_fn::<_, 3, _>(|_| RunQueue::new());
    let mut models = core::array::from_fn::<_, 3, _>(|_| VecDeque::with_capacity(8));
    let mut random = 71_u64;
    without_heap(|| {
        for _ in 0..4096 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let source = (random >> 12) as usize % 3;
            let tid = (random >> 24) as usize % 8;
            let entry = key(tid);
            match random % 4 {
                0 => {
                    let linked = models.iter().any(|queue| queue.contains(&tid));
                    let result = queues[source].push_back(&mut store, entry);
                    assert_eq!(result.is_ok(), !linked);
                    if !linked {
                        models[source].push_back(tid);
                    }
                }
                1 => assert_eq!(
                    queues[source].pop_front(&mut store).unwrap().map(|key| key.tid),
                    models[source].pop_front()
                ),
                2 => {
                    let position = models[source].iter().position(|id| *id == tid);
                    assert_eq!(
                        queues[source].remove(&mut store, entry).is_ok(),
                        position.is_some()
                    );
                    if let Some(position) = position {
                        models[source].remove(position);
                    }
                }
                _ => {
                    let destination = (source + 1) % 3;
                    let position = models[source].iter().position(|id| *id == tid);
                    let (first, rest) = queues.split_at_mut(source.max(destination));
                    let (src, dst) = if source < destination {
                        (&mut first[source], &mut rest[0])
                    } else {
                        (&mut rest[0], &mut first[destination])
                    };
                    assert_eq!(src.transfer_to(dst, &mut store, entry).is_ok(), position.is_some());
                    if let Some(position) = position {
                        models[source].remove(position);
                        models[destination].push_back(tid);
                    }
                }
            }
            for (queue, model) in queues.iter().zip(&models) {
                assert_eq!(queue.len(), model.len());
                let mut actual = queue.iter(&store);
                for tid in model {
                    assert_eq!(actual.next(), Some(Ok(key(*tid))));
                }
                assert_eq!(actual.next(), None);
            }
        }
    });
}
