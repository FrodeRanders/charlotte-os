//! Serialized real observer backing pressure. No root or waiter is fabricated
//! from a numeric identity; the scheduler supplies its prepared fixture owner.
use core::{
    alloc::Allocator as _,
    sync::atomic::Ordering,
};

use super::*;
use crate::{
    klib::observer::{
        Observer,
        ObserverRef,
        ObserverWeak,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

static HITS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static DROPS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
struct Probe;
impl Observer for Probe {
    fn notify(&self) {
        HITS.fetch_add(1, Ordering::Relaxed);
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::Relaxed);
    }
}
struct Large([u8; 1024]);
impl Observer for Large {
    fn notify(&self) {
        core::hint::black_box(self.0[0]);
    }
}

fn small(platform: bool) -> Result<ObserverWeak, AllocError> {
    let observer = ObserverRef::try_new(Probe, platform)?;
    Ok(observer.downgrade().erase())
}
fn large(platform: bool) -> Result<ObserverWeak, AllocError> {
    let observer = ObserverRef::try_new(Large([0; 1024]), platform)?;
    Ok(observer.downgrade().erase())
}

fn deny_layout(platform: bool, bytes: u64) {
    let before = snapshot();
    let owner = allocator(platform).unwrap();
    let held = snapshot();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let layout = Layout::from_size_align(bytes as usize, 1).unwrap();
        assert!(owner.allocate(layout).is_err());
    }
    assert_eq!(snapshot(), held);
    drop(owner);
    assert_eq!(snapshot(), before);
}

pub(crate) fn test_admission(mut real_waker_denial: impl FnMut()) {
    let before = snapshot();
    let strong = ObserverRef::try_new(Probe, false).unwrap();
    let weak = strong.downgrade().erase();
    let live = snapshot();
    let probe_bytes = live.0[0] - before.0[0];
    assert_eq!(live.0[1], before.0[1] + 1);
    assert_eq!(live.1, [before.1[0] + probe_bytes, before.1[1] + 1]);
    let strong = {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let mut erased = None;
        with_locked(|| {
            let owned = strong.erase();
            let upgrade = weak.upgrade().unwrap();
            assert!(ObserverRef::ptr_eq(&owned, &upgrade));
            let extra = owned.clone();
            drop((upgrade, extra));
            erased = Some(owned);
        });
        erased.unwrap()
    };
    let hits = HITS.load(Ordering::Acquire);
    strong.notify();
    assert_eq!(HITS.load(Ordering::Acquire), hits + 1);
    let drops = DROPS.load(Ordering::Acquire);
    drop(strong);
    assert_eq!(DROPS.load(Ordering::Acquire), drops + 1);
    assert!(weak.upgrade().is_none());
    assert_eq!(snapshot(), live);
    drop(weak);
    assert_eq!(snapshot(), before);

    let mut ordinary = alloc::vec::Vec::with_capacity(ORDINARY[1] as usize);
    while let Ok(observer) = small(false) {
        ordinary.push(observer);
        assert!(ordinary.len() <= ORDINARY[1] as usize);
    }
    let full_ordinary = snapshot();
    assert_eq!(full_ordinary.1[1], ORDINARY[1]);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            assert!(small(false).is_err());
        }
    }
    assert_eq!(snapshot(), full_ordinary);
    let trusted = small(true).unwrap();
    assert_eq!(
        snapshot(),
        ([full_ordinary.0[0] + probe_bytes, full_ordinary.0[1] + 1], full_ordinary.1)
    );
    drop(trusted);
    assert_eq!(snapshot(), full_ordinary);
    let mut trusted = alloc::vec::Vec::with_capacity((LIMIT[1] - ORDINARY[1]) as usize);
    while let Ok(observer) = small(true) {
        trusted.push(observer);
        assert!(trusted.len() <= (LIMIT[1] - ORDINARY[1]) as usize);
    }
    let full = snapshot();
    assert_eq!(full.0[1], LIMIT[1]);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            assert!(small(true).is_err());
        }
    }
    real_waker_denial();
    assert_eq!(snapshot(), full);
    drop(ordinary.pop().unwrap());
    ordinary.push(small(false).unwrap());
    assert_eq!(snapshot(), full);
    let count_pressure = (ordinary.len(), trusted.len());
    drop((ordinary, trusted));
    assert_eq!(snapshot(), before);

    let mut ordinary = alloc::vec::Vec::with_capacity(ORDINARY[1] as usize);
    let first = large(false).unwrap();
    let large_bytes = snapshot().0[0] - before.0[0];
    ordinary.push(first);
    while let Ok(observer) = large(false) {
        ordinary.push(observer);
        assert!(ordinary.len() < ORDINARY[1] as usize);
    }
    let bytes_ordinary = snapshot();
    assert!(bytes_ordinary.1[1] < ORDINARY[1]);
    assert!(ORDINARY[0] - bytes_ordinary.1[0] < large_bytes);
    assert!(large(false).is_err());
    assert_eq!(snapshot(), bytes_ordinary);
    let ordinary_denial = ORDINARY[0] - bytes_ordinary.1[0] + 1;
    assert!(LIMIT[0] - bytes_ordinary.0[0] > ordinary_denial);
    deny_layout(false, ordinary_denial);
    let mut trusted = alloc::vec::Vec::with_capacity((LIMIT[1] - ORDINARY[1]) as usize);
    while let Ok(observer) = large(true) {
        trusted.push(observer);
        assert!(trusted.len() < (LIMIT[1] - ORDINARY[1]) as usize);
    }
    let bytes_full = snapshot();
    assert!(bytes_full.0[1] < LIMIT[1]);
    assert!(LIMIT[0] - bytes_full.0[0] < large_bytes);
    deny_layout(true, LIMIT[0] - bytes_full.0[0] + 1);
    assert_eq!(snapshot(), bytes_full);
    let byte_pressure = (ordinary.len(), trusted.len());
    drop((ordinary, trusted));
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[observer backing] actual Arc layouts probe={} large={}; weak-only count pressure \
         ordinary={} trusted={} at {}/{}; real byte pressure ordinary={} trusted={} below count \
         ceilings; 128 heap-held bundle denials, payload-layout byte denials, guarded \
         erasure/upgrade, trusted reserve, exact weak-release recovery and real waker rejection \
         passed",
        probe_bytes,
        large_bytes,
        count_pressure.0,
        count_pressure.1,
        ORDINARY[1],
        LIMIT[1],
        byte_pressure.0,
        byte_pressure.1
    );
}
