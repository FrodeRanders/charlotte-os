//! Host regression tests for the raw shared-page ABI, including concurrent slot reuse.
use core::{
    cell::UnsafeCell,
    sync::atomic::{
        AtomicU32,
        Ordering,
    },
};
use std::{
    sync::Arc,
    thread,
    time::{
        Duration,
        Instant,
    },
};

use super::{
    CompletionQueueRing,
    CqEntry,
    cq_read,
};

struct Page(UnsafeCell<[u64; 512]>);
// SAFETY: tests initialize before sharing, then use exactly one producer and
// one consumer. Cursor atomics transfer exclusive access to each payload slot.
unsafe impl Sync for Page {}
impl Page {
    fn new(capacity: u32) -> Self {
        let page = Self(UnsafeCell::new([0; 512]));
        // SAFETY: exclusively owned, aligned full-page backing before sharing.
        unsafe {
            page.ring().write(CompletionQueueRing {
                head: AtomicU32::new(0),
                tail: AtomicU32::new(0),
                capacity: AtomicU32::new(capacity),
                overflow: AtomicU32::new(0),
                entries: [],
            });
        }
        page
    }

    fn ring(&self) -> *mut CompletionQueueRing {
        self.0.get().cast()
    }

    fn base(&self) -> usize {
        self.ring() as usize
    }
}
fn record(i: u64) -> CqEntry {
    CqEntry {
        operation: i,
        cookie: !i,
        status: (i % 3) as u32,
        flags: 0,
        result: -(i as i64),
    }
}
#[test]
fn wire_layout_is_unchanged() {
    assert_eq!(core::mem::size_of::<CompletionQueueRing>(), 16);
    assert_eq!(core::mem::align_of::<CompletionQueueRing>(), 8);
    assert_eq!(core::mem::offset_of!(CompletionQueueRing, head), 0);
    assert_eq!(core::mem::offset_of!(CompletionQueueRing, tail), 4);
    assert_eq!(core::mem::offset_of!(CompletionQueueRing, capacity), 8);
    assert_eq!(core::mem::offset_of!(CompletionQueueRing, overflow), 12);
    assert_eq!(core::mem::offset_of!(CompletionQueueRing, entries), 16);
    assert_eq!(core::mem::size_of::<CqEntry>(), 32);
}
#[test]
fn malformed_headers_are_rejected_without_advancing_tail() {
    let page = Page::new(4);
    // SAFETY: live mapping with an exclusive test consumer; null and misaligned
    // values are rejected before dereference. Header mutations are quiescent.
    unsafe {
        assert_eq!(cq_read(0, 4), None);
        assert_eq!(cq_read(page.base() + 1, 4), None);
        let ring = &*page.ring();
        for capacity in [0, 1, 5, 128, u32::MAX] {
            ring.capacity.store(capacity, Ordering::Relaxed);
            assert_eq!(cq_read(page.base(), 4), None);
            assert_eq!(ring.tail.load(Ordering::Relaxed), 0);
        }
        ring.capacity.store(4, Ordering::Relaxed);
        for (head, tail) in [(4, 0), (1, 4), (u32::MAX, 0), (0, u32::MAX)] {
            ring.head.store(head, Ordering::Relaxed);
            ring.tail.store(tail, Ordering::Relaxed);
            assert_eq!(cq_read(page.base(), 4), None);
            assert_eq!(ring.tail.load(Ordering::Relaxed), tail);
        }
    }
}
#[test]
fn concurrent_producer_and_sdk_consumer_wrap_and_reuse_slots() {
    // Capacity two forces a handoff for every record, maximizing slot reuse.
    for capacity in [2, 3, 32, 127] {
        let page = Arc::new(Page::new(capacity));
        let producer_page = Arc::clone(&page);
        let deadline = Instant::now() + Duration::from_secs(10);
        let producer = thread::spawn(move || {
            // SAFETY: producer retains backing, owns only head/free slots, and
            // acquires tail before reuse. The SDK is the sole tail/entry reader.
            let ring = unsafe { &*producer_page.ring() };
            for i in 0..20_000 {
                let head = ring.head.load(Ordering::Relaxed);
                let next = (head + 1) % capacity;
                while next == ring.tail.load(Ordering::Acquire) {
                    assert!(Instant::now() < deadline, "producer stalled");
                    thread::yield_now();
                }
                // SAFETY: acquired tail makes this slot exclusively writable.
                unsafe {
                    core::ptr::addr_of!((*producer_page.ring()).entries)
                        .cast::<CqEntry>()
                        .cast_mut()
                        .add(head as usize)
                        .write_volatile(record(i));
                }
                ring.head.store(next, Ordering::Release);
            }
        });
        for i in 0..20_000 {
            let entry = loop {
                // SAFETY: this is the sole consumer of the retained full page.
                if let Some(entry) = unsafe { cq_read(page.base(), capacity) } {
                    break entry;
                }
                assert!(Instant::now() < deadline, "consumer stalled");
                thread::yield_now();
            };
            assert_eq!(entry, record(i));
        }
        producer.join().unwrap();
        // SAFETY: sole consumer, producer has joined, and backing is retained.
        assert_eq!(unsafe { cq_read(page.base(), capacity) }, None);
    }
}
