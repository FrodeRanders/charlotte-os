//! Observe actual buffer frees while production charges are still retained.
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
static BYTES_AT_FREE: AtomicU64 = AtomicU64::new(0);
#[global_allocator]
static ALLOCATOR: Tracer = Tracer;
unsafe impl GlobalAlloc for Tracer {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if pointer as usize == WATCH.load(Ordering::Acquire) {
            BYTES_AT_FREE.store(admission::used::<IommuNodes, 2>().0[0], Ordering::Release);
            FREES.fetch_add(1, Ordering::AcqRel);
        }
        unsafe { System.dealloc(pointer, layout) }
    }
}
fn used() -> ([u64; 2], [u64; 2]) {
    admission::used::<IommuNodes, 2>()
}
fn watch(regions: &[Region]) {
    FREES.store(0, Ordering::Release);
    WATCH.store(regions.as_ptr() as usize, Ordering::Release);
}
fn freed(bytes: u64) {
    WATCH.store(0, Ordering::Release);
    assert_eq!(FREES.load(Ordering::Acquire), 1);
    assert_eq!(BYTES_AT_FREE.load(Ordering::Acquire), bytes);
}
fn filled() -> Ledger {
    let mut ledger = Ledger::new();
    ledger.prepare(false, 8).unwrap();
    for base in 0..4 {
        ledger.push(Region {
            base,
            pages: base + 1,
        });
    }
    ledger
}
fn allocate(regions: &mut Vec<Region>, capacity: usize) -> Result<(), Error> {
    regions.try_reserve_exact(capacity).map_err(|_| Error::MapFailed)
}
#[test]
fn growth_holds_full_overlap_until_old_buffer_is_freed() {
    let before = used();
    let mut ledger = filled();
    watch(&ledger.0.regions);
    ledger
        .prepare_with(
            false,
            8,
            |regions, capacity| {
                assert_eq!(used().0[0], before.0[0] + 192);
                allocate(regions, capacity)
            },
            || freed(before.0[0] + 192),
        )
        .unwrap();
    assert_eq!(used().0, [before.0[0] + 128, before.0[1]]);
    for (i, region) in ledger.iter().enumerate() {
        assert_eq!((region.base, region.pages), (i, i + 1));
    }
    watch(&ledger.0.regions);
    ledger.release();
    freed(before.0[0] + 128);
    assert_eq!(used(), before);
}
#[test]
fn allocated_candidate_rejection_frees_before_refund_and_preserves_old_records() {
    let before = used();
    let mut ledger = filled();
    let old = ledger.0.regions.as_ptr();
    assert_eq!(
        ledger.prepare_with(
            false,
            8,
            |regions, capacity| {
                allocate(regions, capacity)?;
                watch(regions);
                Err(Error::MapFailed)
            },
            || panic!("rejection replaced old ledger")
        ),
        Err(Error::MapFailed)
    );
    freed(before.0[0] + 192);
    assert_eq!(used().0[0], before.0[0] + 64);
    assert_eq!(ledger.0.regions.as_ptr(), old);
    assert_eq!(ledger.iter().map(|r| r.base).collect::<Vec<_>>(), [0, 1, 2, 3]);
    ledger.release();
    assert_eq!(used(), before);
}
#[test]
fn exact_capacity_and_owner_bound_reject_without_publication() {
    let before = used();
    let mut ledger = filled();
    assert_eq!(
        ledger.prepare_with(false, 8, |regions, capacity| allocate(regions, capacity - 1), || {}),
        Err(Error::MapFailed)
    );
    assert_eq!(ledger.capacity(), 4);
    assert_eq!(
        ledger.prepare_with(false, 4, |_, _| panic!("bound reached allocator"), || {}),
        Err(Error::MapFailed)
    );
    ledger.release();
    assert_eq!(used(), before);
}
#[test]
fn existing_capacity_progresses_under_byte_pressure_without_new_allocation() {
    let before = used();
    let mut ledger = Ledger::new();
    ledger.prepare(false, 8).unwrap();
    let ceiling = crate::resource_policy::POLICY.iommu_nodes.ordinary[0];
    let pressure = Charge::reserve(false, [ceiling - used().1[0], 0]).unwrap();
    ledger.prepare_with(false, 8, |_, _| panic!("unused capacity allocated"), || {}).unwrap();
    assert!(Ledger::new().prepare(false, 8).is_err());
    let trusted = Backing::prepare(true, 4, allocate).unwrap();
    assert_eq!(used().1[0], ceiling);
    trusted.finish(|| {});
    drop(pressure);
    ledger.release();
    assert_eq!(used(), before);
}
#[test]
fn abandonment_under_pool_lock_keeps_buffer_and_original_charge() {
    let before = used();
    let ledger = filled();
    watch(&ledger.0.regions);
    admission::with_locked::<IommuNodes, 2, _>(|| drop(ledger));
    WATCH.store(0, Ordering::Release);
    assert_eq!(FREES.load(Ordering::Acquire), 0);
    assert_eq!(used(), ([before.0[0] + 64, before.0[1]], [before.1[0] + 64, before.1[1]]));
}
