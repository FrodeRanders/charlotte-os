//! Native fixed-cache and store response controls.
use alloc::vec::Vec;

use super::*;
use crate::klib::admission::{
    self,
    Family,
    Reservation,
};

pub(crate) fn run() {
    for (result, memory, connection, expected) in [
        (1, false, false, Ok(None)),
        (4, false, false, Err(ReadError::Unavailable)),
        (7, false, false, Err(ReadError::Unavailable)),
        (-1, false, false, Err(ReadError::Unavailable)),
        (1, true, false, Ok(Some(1))),
        (0, true, false, Ok(None)),
        (-1, true, false, Err(ReadError::Malformed)),
        (1, false, true, Err(ReadError::Malformed)),
    ] {
        assert_eq!(read_length(result, memory, connection), expected);
    }
    let cache = Cache::<1>::new();
    let before = admission::used::<ServiceImages, 2>();
    let mut prepared = cache.prepare(b"fixture").unwrap();
    let mut image = Vec::new_in(ByteAllocator::<ServiceImages>::new(true));
    image.try_reserve_exact(37).unwrap();
    image.resize(37, 9);
    prepared.image = Some(image);
    assert!(cache.get(b"fixture").is_none());
    prepared.cancel();
    assert_eq!(admission::used::<ServiceImages, 2>(), before);
    let pressure =
        Reservation::<ServiceImages, 2>::reserve(true, [0, ServiceImages::TOTAL[1] - before.0[1]])
            .unwrap();
    {
        let _heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        assert_eq!(cache.prepare(b"denied").err(), Some(image_cache::AdmissionError::Full));
    }
    drop(pressure);
    static ABANDONED: [Cache<1>; 4] = [const { Cache::new() }; 4];
    for (index, cache) in ABANDONED.iter().enumerate() {
        let mut prepared = cache.prepare(b"retained").unwrap();
        if index >= 2 {
            let mut image = Vec::new_in(ByteAllocator::<ServiceImages>::new(true));
            image.try_reserve_exact(37).unwrap();
            image.resize(37, 9);
            prepared.image = Some(image);
        }
        let charged = admission::used::<ServiceImages, 2>();
        let irq = crate::cpu::isa::lp::ops::get_int_state();
        if index.is_multiple_of(2) {
            drop(prepared);
        } else {
            let _heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
            cache.with_locked(|| admission::with_locked::<ServiceImages, 2, _>(|| drop(prepared)));
        }
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
        assert_eq!(admission::used::<ServiceImages, 2>(), charged);
        assert!(cache.get(b"retained").is_none());
        assert_eq!(cache.prepare(b"retained").err(), Some(image_cache::AdmissionError::Duplicate));
    }
    crate::logln!(
        "[service image cache abandonment] four count reservations and two 37-byte private images \
         retained before/after copy; heap/cache/pool guards and IRQ preservation passed"
    );
    crate::logln!(
        "[service image cache] reserved publication, private cancellation and \
         absent/error/attachment response discrimination passed"
    );
}

pub(crate) fn live() {
    let first = load_cached_elf(b"ns").expect("native cache read");
    assert!(verified_for_name(b"ns", first));
    // Steady-state boot seeding has completed on this caller. Hold the
    // single-flight gate against every other cache-miss producer across the
    // global-pool pressure/seed/read observations; a local IRQ mask cannot do it.
    let deadline = crate::cpu::scheduler::monotonic_millis().saturating_add(3000);
    let _reader = loop {
        if STORE_READER_ACTIVE
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            break StoreReaderGuard;
        }
        assert!(crate::cpu::scheduler::monotonic_millis() < deadline);
        crate::cpu::scheduler::yield_lp();
    };
    let before = admission::used::<ServiceImages, 2>();
    {
        let _heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        admission::with_locked::<ServiceImages, 2, _>(|| {
            assert_eq!(cached_elf(b"ns").unwrap().as_ptr(), first.as_ptr())
        });
    }
    assert_eq!(admission::used::<ServiceImages, 2>(), before);
    let pressure =
        Reservation::<ServiceImages, 2>::reserve(true, [ServiceImages::TOTAL[0] - before.0[0], 0])
            .unwrap();
    assert_eq!(seed_embedded_services(), Err("existing service image could not be read"));
    drop(pressure);
    let read = read_from_store(b"ns").unwrap().unwrap();
    assert_eq!(&*read, first);
    drop(read);
    assert_eq!(admission::used::<ServiceImages, 2>(), before);
    crate::logln!(
        "[service image cache live] signed store read={} bytes, immutable allocation-free hit \
         under heap/pool guards, quota-rejected reseed stops and installed image remains \
         unchanged; exact temporary-byte recovery",
        first.len()
    );
}
