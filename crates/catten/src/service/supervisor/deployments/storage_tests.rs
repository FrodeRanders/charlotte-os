use std::{
    alloc::{
        GlobalAlloc,
        Layout,
        System,
    },
    cell::Cell,
    sync::{
        Arc,
        Barrier,
        Mutex,
        atomic::{
            AtomicUsize,
            Ordering,
        },
    },
};
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
fn complete_registry_cycles_allocate_and_deallocate_nothing() {
    HEAP_CALLS.with(|calls| calls.set(Some((0, 0))));
    for _ in 0..256 {
        let mut registry = Registry::<u64, 2>::new();
        let first = registry.reserve(1).unwrap();
        let other = registry.reserve(2).unwrap();
        assert_eq!(registry.reserve(3).unwrap_err(), Error::Full);
        registry.cancel(other);
        registry.publish(first, 1);
        assert_eq!(registry.remove(0), 1);
        // An abandoned preparation/registry must not call an allocator either.
        let _abandoned = registry.reserve(4).unwrap();
    }
    let calls = HEAP_CALLS.with(|calls| calls.replace(None)).unwrap();
    assert_eq!(calls, (0, 0));
}

use super::*;

#[test]
fn uncommitted_launches_consume_capacity_and_fence_duplicate_principals() {
    let mut registry = Registry::<u64, 64>::new();
    let mut reservations = Vec::new();
    assert_eq!(registry.reserve(0).unwrap_err(), Error::InvalidPrincipal);
    for principal in 1..=64 {
        reservations.push(registry.reserve(principal).unwrap());
        assert_eq!(registry.reserve(principal).unwrap_err(), Error::Duplicate);
    }
    assert_eq!(registry.len(), 0);
    assert_eq!(registry.reserve(65).unwrap_err(), Error::Full);
    for reservation in reservations {
        let principal = reservation.principal();
        registry.publish(reservation, principal);
    }
    assert_eq!(registry.len(), 64);
    assert_eq!(registry.reserve(1).unwrap_err(), Error::Duplicate);
    assert_eq!(registry.reserve(65).unwrap_err(), Error::Full);
}

#[test]
fn rejected_launch_releases_only_its_exact_preparation_and_serial_is_never_reused() {
    let mut registry = Registry::<u64, 2>::new();
    let first = registry.reserve(1).unwrap();
    let stale = Reservation {
        index: first.index,
        principal: first.principal,
        serial: first.serial,
    };
    let other = registry.reserve(2).unwrap();
    registry.cancel(first);
    let replacement = registry.reserve(1).unwrap();
    assert_eq!(replacement.index, stale.index);
    assert_ne!(replacement.serial, stale.serial);
    assert!(!registry.matches(&stale));
    assert!(registry.matches(&replacement));
    assert!(registry.matches(&other));
    registry.cancel(replacement);
    registry.cancel(other);
    registry.next_serial = u64::MAX;
    assert_eq!(registry.reserve(3).unwrap_err(), Error::Exhausted);
    assert!(!registry.preparing(3));
}

#[test]
fn removal_keeps_other_preparations_and_published_positions_stable() {
    let mut registry = Registry::<u64, 3>::new();
    let first = registry.reserve(1).unwrap();
    let second = registry.reserve(2).unwrap();
    let third = registry.reserve(3).unwrap();
    registry.publish(first, 11);
    registry.publish(third, 33);
    assert_eq!(registry.remove(0), 11);
    assert_eq!(registry.position(|value| *value == 33), Some(2));
    assert_eq!(registry[2], 33);
    assert!(registry.preparing(2));
    registry.publish(second, 22);
    registry[2] = 44;
    assert_eq!(registry.iter_mut().map(|v| *v).collect::<Vec<_>>(), [22, 44]);
    assert_eq!(registry.remove(1), 22);
    assert_eq!(registry.remove(2), 44);
    assert_eq!(registry.len(), 0);
}

struct Payload(Arc<AtomicUsize>);
impl Drop for Payload {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn abandon<T>(_: T) {}

#[test]
fn abandoned_preparation_and_registry_never_release_live_payloads() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut registry = Registry::<Payload, 2>::new();
    let abandoned = registry.reserve(1).unwrap();
    abandon(abandoned); // End its lifetime before checking the exact fence.
    assert!(registry.preparing(1));
    assert_eq!(registry.reserve(1).unwrap_err(), Error::Duplicate);
    let second = registry.reserve(2).unwrap();
    registry.publish(second, Payload(drops.clone()));
    assert_eq!(registry.reserve(3).unwrap_err(), Error::Full);
    drop(registry);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
}

#[test]
fn explicit_removal_returns_the_complete_payload_before_disposal() {
    let drops = Arc::new(AtomicUsize::new(0));
    let registry = Mutex::new(Registry::<Payload, 1>::new());
    let payload = {
        let mut registry = registry.lock().unwrap();
        registry.insert(1, Payload(drops.clone()));
        registry.remove(0)
    };
    assert!(registry.try_lock().is_ok());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(payload);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn concurrent_callers_cannot_overbook_capacity_or_duplicate_a_preparing_principal() {
    for same_principal in [false, true] {
        let registry = Arc::new(Mutex::new(Registry::<usize, 4>::new()));
        let barrier = Arc::new(Barrier::new(16));
        let admitted = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        for id in 0..16 {
            let (registry, barrier, admitted) =
                (registry.clone(), barrier.clone(), admitted.clone());
            workers.push(std::thread::spawn(move || {
                let principal = if same_principal {
                    1
                } else {
                    id as u64 + 1
                };
                let reservation = registry.lock().unwrap().reserve(principal).ok();
                if reservation.is_some() {
                    admitted.fetch_add(1, Ordering::SeqCst);
                }
                // All preflights finish before any launch publication, the
                // interleaving that escaped the former vector-only checks.
                barrier.wait();
                if let Some(reservation) = reservation {
                    registry.lock().unwrap().publish(reservation, id);
                }
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
        let expected = if same_principal {
            1
        } else {
            4
        };
        assert_eq!(admitted.load(Ordering::SeqCst), expected);
        assert_eq!(registry.lock().unwrap().len(), expected);
    }
}
