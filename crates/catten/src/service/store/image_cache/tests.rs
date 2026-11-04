//! Host controls for the production fixed cache.
use core::{
    alloc::Layout,
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

use super::*;
use crate::klib::admission::{
    self,
    Family,
};
static WATCH: AtomicUsize = AtomicUsize::new(0);
static BYTES_AT_FREE: AtomicU64 = AtomicU64::new(0);
static COUNT_AT_FREE: AtomicU64 = AtomicU64::new(0);
static REJECT: AtomicBool = AtomicBool::new(false);
struct Tracer;
#[global_allocator]
static GLOBAL: Tracer = Tracer;
unsafe impl GlobalAlloc for Tracer {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 129 && REJECT.swap(false, Ordering::AcqRel) {
            return core::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        if WATCH.load(Ordering::Acquire) == pointer as usize {
            let used = used().0;
            BYTES_AT_FREE.store(used[0], Ordering::Release);
            COUNT_AT_FREE.store(used[1], Ordering::Release);
        }
    }
}
fn used() -> ([u64; 2], [u64; 2]) {
    admission::used::<ServiceImages, 2>()
}
fn image(len: usize) -> Image {
    let mut image = Image::new_in(ByteAllocator::new(true));
    image.try_reserve_exact(len).unwrap();
    image.resize(len, 0x5a);
    image
}
#[test]
fn reserve_publish_duplicate_and_disposal_preserve_allocation_lifetime() {
    let before = used();
    let cache = Cache::<2>::new();
    let mut preparing = cache.prepare(b"one").unwrap();
    assert!(cache.get(b"one").is_none());
    assert_eq!(cache.prepare(b"one").err(), Some(AdmissionError::Duplicate));
    preparing.image = Some(image(129));
    let bytes = preparing.publish();
    assert_eq!(bytes, &[0x5a; 129]);
    assert_eq!(cache.get(b"one").unwrap().as_ptr(), bytes.as_ptr());
    assert_eq!(used(), ([before.0[0] + 129, before.0[1] + 1], before.1));
    WATCH.store(bytes.as_ptr() as usize, Ordering::Release);
    drop(cache);
    WATCH.store(0, Ordering::Release);
    assert_eq!(BYTES_AT_FREE.load(Ordering::Acquire), before.0[0] + 129);
    assert_eq!(COUNT_AT_FREE.load(Ordering::Acquire), before.0[1] + 1);
    assert_eq!(used(), before);
}
#[test]
fn backend_error_retains_reserved_slot_until_explicit_cancellation() {
    let before = used();
    let cache = Cache::<1>::new();
    let preparing = cache.prepare(b"one").unwrap();
    let charged = used();
    let mut candidate = Image::new_in(ByteAllocator::new(true));
    REJECT.store(true, Ordering::Release);
    assert!(candidate.try_reserve_exact(129).is_err());
    assert!(!REJECT.load(Ordering::Acquire));
    assert_eq!(used(), charged);
    assert_eq!(cache.prepare(b"other").err(), Some(AdmissionError::Full));
    assert_eq!(used(), charged);
    preparing.cancel();
    assert_eq!(used(), before);
}
#[test]
fn actual_maximum_images_exhaust_bytes_before_counts_and_recover_exactly() {
    let before = used();
    let cache = Cache::<5>::new();
    let mut a = cache.prepare(b"a").unwrap();
    a.image = Some(image(4 * 1024 * 1024));
    a.publish();
    let mut b = cache.prepare(b"b").unwrap();
    b.image = Some(image(4 * 1024 * 1024));
    b.publish();
    let mut c = cache.prepare(b"c").unwrap();
    c.image = Some(image(4 * 1024 * 1024));
    c.publish();
    let remaining = (ServiceImages::TOTAL[0] - used().0[0]) as usize;
    let mut d = cache.prepare(b"d").unwrap();
    d.image = Some(image(remaining));
    d.publish();
    assert_eq!(used().0[0], ServiceImages::TOTAL[0]);
    assert!(used().0[1] < ServiceImages::TOTAL[1]);
    let e = cache.prepare(b"e").unwrap();
    assert!(Image::new_in(ByteAllocator::new(true)).try_reserve_exact(1).is_err());
    e.cancel();
    drop(cache);
    assert_eq!(used(), before);
}
#[test]
fn private_cancellation_frees_bytes_before_refunding_the_reserved_slot() {
    let before = used();
    let cache = Cache::<1>::new();
    let mut preparing = cache.prepare(b"one").unwrap();
    preparing.image = Some(image(129));
    WATCH.store(preparing.image.as_ref().unwrap().as_ptr() as usize, Ordering::Release);
    preparing.cancel();
    WATCH.store(0, Ordering::Release);
    assert_eq!(BYTES_AT_FREE.load(Ordering::Acquire), before.0[0] + 129);
    assert_eq!(COUNT_AT_FREE.load(Ordering::Acquire), before.0[1] + 1);
    assert_eq!(used(), before);
    cache.prepare(b"one").unwrap().cancel();
}
#[test]
fn count_denial_precedes_preparation_and_abandonment_retains_the_complete_prefix() {
    let before = used();
    let cache = Cache::<1>::new();
    let pressure =
        Reservation::<ServiceImages, 2>::reserve(true, [0, ServiceImages::TOTAL[1] - before.0[1]])
            .unwrap();
    let charged = used();
    assert_eq!(cache.prepare(b"one").err(), Some(AdmissionError::Full));
    assert_eq!(used(), charged);
    drop(pressure);
    let mut preparing = cache.prepare(b"one").unwrap();
    preparing.image = Some(image(129));
    let charged = used();
    drop(preparing);
    assert_eq!(used(), charged, "abandonment released the complete image prefix");
    assert_eq!(cache.prepare(b"one").err(), Some(AdmissionError::Duplicate));
    assert!(cache.get(b"one").is_none());
    drop(cache);
    assert_eq!(used(), charged);
}
