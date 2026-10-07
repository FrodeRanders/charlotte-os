use core::sync::atomic::AtomicUsize;
use std::{
    sync::Barrier,
    thread,
};

use super::*;

struct Charge(Arc<AtomicUsize>);
impl Charge {
    fn new(used: &Arc<AtomicUsize>) -> Self {
        used.fetch_add(1, Ordering::SeqCst);
        Self(used.clone())
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        assert_eq!(self.0.fetch_sub(1, Ordering::SeqCst), 1);
    }
}
struct Payload {
    used: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}
impl Drop for Payload {
    fn drop(&mut self) {
        assert_eq!(self.used.load(Ordering::SeqCst), 1, "payload must drop before refund");
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn strong_and_weak_storage_return_one_original_charge() {
    let used = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
    let value = Arc::try_new_in(
        Payload {
            used: used.clone(),
            dropped: dropped.clone(),
        },
        allocator,
    )
    .unwrap();
    let weak = Arc::downgrade(&value);
    let aliases: Vec<_> = (0..128).map(|_| weak.clone()).collect();
    let upgraded = weak.upgrade().unwrap();
    drop(value);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(upgraded);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(weak.upgrade().is_none());
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(aliases);
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(weak);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn allocation_owner_failure_returns_unused_charge() {
    let used = Arc::new(AtomicUsize::new(0));
    assert!(ChargedAllocator::try_new_with(Charge::new(&used), |_| Err(AllocError)).is_err());
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn cloned_allocators_cannot_multiply_or_revive_allocations() {
    let used = Arc::new(AtomicUsize::new(0));
    let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
    let value = Arc::try_new_in(17u64, allocator.clone()).unwrap();
    assert!(Arc::try_new_in(18u64, allocator.clone()).is_err());
    assert_eq!(*value, 17);
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(value);
    assert_eq!(used.load(Ordering::SeqCst), 1);
    assert!(Arc::try_new_in(19u64, allocator.clone()).is_err());
    drop(allocator);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

struct RejectAllocation<A>(A);
// SAFETY: Refuses every allocation; any forwarded deallocation belongs to A.
unsafe impl<A: Allocator> Allocator for RejectAllocation<A> {
    fn allocate(&self, _: Layout) -> Result<NonNull<[u8]>, AllocError> {
        Err(AllocError)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { self.0.deallocate(ptr, layout) };
    }
}

#[test]
fn payload_allocation_failure_drops_payload_before_charge() {
    let used = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
    let rejected = allocator.try_arc_with(
        Payload {
            used: used.clone(),
            dropped: dropped.clone(),
        },
        |value, allocator| match Arc::try_new_in(value, RejectAllocation(allocator)) {
            Err(error) => Err(error),
            Ok(_) => panic!("injected allocation unexpectedly succeeded"),
        },
    );
    assert!(rejected.is_err());
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn concurrent_last_weak_destruction_refunds_once() {
    for _ in 0..64 {
        let used = Arc::new(AtomicUsize::new(0));
        let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
        let value = Arc::try_new_in(23u64, allocator).unwrap();
        let weak = Arc::downgrade(&value);
        let barrier = Arc::new(Barrier::new(4));
        let joins: Vec<_> = (0..4)
            .map(|_| {
                let weak = weak.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    drop(weak);
                })
            })
            .collect();
        drop(value);
        assert_eq!(used.load(Ordering::SeqCst), 1);
        drop(weak);
        for join in joins {
            join.join().unwrap();
        }
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn concurrent_allocator_clones_admit_only_one_backing() {
    let used = Arc::new(AtomicUsize::new(0));
    let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
    let barrier = Arc::new(Barrier::new(4));
    let joins: Vec<_> = (0..4)
        .map(|number| {
            let allocator = allocator.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                allocator.try_arc(number)
            })
        })
        .collect();
    let outcomes: Vec<_> = joins.into_iter().map(|join| join.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(outcomes);
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(allocator);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}
