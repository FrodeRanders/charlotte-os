//! Serialized real backing pressure through each production source factory.
use super::*;
use crate::{
    cpu::scheduler::deferred,
    klib::observer::{
        list_budget,
        registration::ObserverList,
        waiter_budget,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

pub(crate) fn test_source<T, E>(
    family: &'static str,
    mut source: impl FnMut(bool) -> Result<T, E>,
    mut cancel: impl FnMut(T),
    mut publish_under_guards: impl FnMut(T),
) {
    deferred::drain();
    let before = snapshot();
    let lists_before = list_budget::node_used();
    for _ in 0..64 {
        assert!(prepare_with(false, |_| Err(AllocError)).is_err());
        assert_eq!(snapshot(), before);
    }
    assert!(reserve(ORDINARY[0] + 1, false).is_err());
    assert!(reserve(LIMIT[0] + 1, true).is_err());
    assert_eq!(snapshot(), before);

    // A real list-admission rejection follows successful disposal allocation.
    // The partial source must free that candidate and its original charge.
    let mut lists = alloc::vec::Vec::with_capacity(list_budget::ORDINARY_LIMIT);
    while let Ok(list) = ObserverList::<waiter_budget::Charge>::try_new(1, false) {
        lists.push(list);
        assert!(lists.len() <= list_budget::ORDINARY_LIMIT);
    }
    assert_eq!(list_budget::node_used().1, list_budget::ORDINARY_LIMIT);
    for _ in 0..64 {
        assert!(source(false).is_err());
        assert_eq!(snapshot(), before);
    }
    drop(lists);
    assert_eq!(list_budget::node_used(), lists_before);

    let bytes = PreparedDisposal::allocation_bytes() as u64;
    let mut ordinary = alloc::vec::Vec::with_capacity(ORDINARY[1] as usize);
    while let Ok(node) = prepare(false) {
        ordinary.push(node);
        assert!(ordinary.len() <= ORDINARY[1] as usize);
    }
    let full_ordinary = snapshot();
    assert_eq!(full_ordinary.1[1], ORDINARY[1]);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            assert!(source(false).is_err());
        }
    }
    assert_eq!(snapshot(), full_ordinary);
    assert_eq!(list_budget::node_used(), lists_before);
    let trusted = source(true).unwrap_or_else(|_| panic!("{} trusted source rejected", family));
    assert_eq!(snapshot(), ([full_ordinary.0[0] + bytes, full_ordinary.0[1] + 1], full_ordinary.1));
    assert_eq!(list_budget::node_used(), (lists_before.0 + 1, lists_before.1));
    cancel(trusted);
    assert_eq!(snapshot(), full_ordinary);
    assert_eq!(list_budget::node_used(), lists_before);

    let mut trusted = alloc::vec::Vec::with_capacity((LIMIT[1] - ORDINARY[1]) as usize);
    while let Ok(node) = prepare(true) {
        trusted.push(node);
        assert!(trusted.len() <= (LIMIT[1] - ORDINARY[1]) as usize);
    }
    let full = snapshot();
    assert_eq!(full.0[1], LIMIT[1]);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            assert!(source(true).is_err());
        }
    }
    assert_eq!(snapshot(), full);
    assert_eq!(list_budget::node_used(), lists_before);

    // Return exactly one ordinary candidate and consume that slot with a real
    // source. Enqueue under the caller's actual producer/heap/pool guards.
    drop(ordinary.pop().unwrap());
    let recovered =
        source(false).unwrap_or_else(|_| panic!("{} recovered source rejected", family));
    assert_eq!(snapshot(), full);
    publish_under_guards(recovered);
    assert_eq!(snapshot(), full, "enqueue refunded live backing");
    assert_eq!(list_budget::node_used(), (lists_before.0 + 1, lists_before.1 + 1));
    deferred::drain();
    assert_eq!(
        snapshot(),
        ([full.0[0] - bytes, full.0[1] - 1], [full.1[0] - bytes, full.1[1] - 1])
    );
    assert_eq!(list_budget::node_used(), lists_before);
    let counts = (ordinary.len(), trusted.len());
    drop((ordinary, trusted));
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[deferred node admission] family={} bytes={} actual candidates ordinary={} trusted={}; \
         ceilings {}/{}; 64 allocator failures, byte denial, 64 partial-list rejections, 128 \
         heap-held source denials, trusted cancellation, guarded enqueue and exact producer-LP \
         drain/refund passed",
        family,
        bytes,
        counts.0,
        counts.1,
        ORDINARY[1],
        LIMIT[1]
    );
}
