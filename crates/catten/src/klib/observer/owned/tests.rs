use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};
use std::{
    sync::{
        Arc,
        Barrier,
    },
    thread,
};

use super::*;

struct Probe {
    hits: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}
impl Observer for Probe {
    fn notify(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}
fn probe() -> (Probe, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    (
        Probe {
            hits: hits.clone(),
            drops: drops.clone(),
        },
        hits,
        drops,
    )
}

#[test]
fn erasure_upgrade_and_borrowed_notification_preserve_identity() {
    let before = backing::snapshot();
    let (payload, hits, drops) = probe();
    let strong = ObserverRef::try_new(payload, false).unwrap();
    let weak = strong.downgrade().erase();
    let erased = strong.erase();
    let upgraded = weak.upgrade().unwrap();
    assert!(ObserverRef::ptr_eq(&erased, &upgraded));
    upgraded.notify();
    assert_eq!(hits.load(Ordering::Acquire), 1);
    drop(upgraded);
    assert_eq!(ObserverRef::strong_count(&erased), 1);
    assert_eq!(ObserverRef::weak_count(&erased), 1);
    let live = backing::snapshot();
    drop(erased);
    assert_eq!(drops.load(Ordering::Acquire), 1);
    assert!(weak.upgrade().is_none());
    assert_eq!(backing::snapshot(), live);
    drop(weak);
    assert_eq!(backing::snapshot(), before);
}
#[test]
fn final_weak_on_another_thread_refunds_original_class_once() {
    let before = backing::snapshot();
    let (payload, _, drops) = probe();
    let strong = ObserverRef::try_new(payload, true).unwrap();
    let weak = strong.downgrade().erase();
    let live = backing::snapshot();
    assert_eq!(live.1, before.1);
    let barrier = Arc::new(Barrier::new(5));
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
    drop(strong);
    drop(weak);
    assert_eq!(drops.load(Ordering::Acquire), 1);
    assert_eq!(backing::snapshot(), live);
    barrier.wait();
    for join in joins {
        join.join().unwrap();
    }
    assert_eq!(backing::snapshot(), before);
}
#[test]
fn empty_weak_owns_no_allocation_and_cannot_upgrade() {
    let before = backing::snapshot();
    let empty: ObserverWeak = ObserverWeak::new();
    let other = empty.clone();
    assert!(empty.upgrade().is_none());
    assert_eq!(empty.strong_count(), 0);
    assert_eq!(empty.weak_count(), 0);
    assert!(empty.ptr_eq(&other));
    drop((empty, other));
    assert_eq!(backing::snapshot(), before);
}
#[test]
fn alignment_padding_is_part_of_the_actual_arc_byte_charge() {
    #[repr(align(4096))]
    struct Aligned;
    impl Observer for Aligned {
        fn notify(&self) {}
    }
    let before = backing::snapshot();
    let observer = ObserverRef::try_new(Aligned, false).unwrap();
    let live = backing::snapshot();
    assert!(live.0[0] - before.0[0] >= 4096);
    assert_eq!((live.0[0] - before.0[0]) % 4096, 0);
    assert_eq!(live.0[1], before.0[1] + 1);
    drop(observer);
    assert_eq!(backing::snapshot(), before);
}
