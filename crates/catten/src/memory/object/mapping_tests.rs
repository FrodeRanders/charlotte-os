//! Same public preparation/lease/pin pipeline with a per-call maintenance
//! adapter. Faults never provide physical completion or a recovery owner.
use super::*;
use crate::{
    cpu::isa::memory::tlb::InvalidationError,
    memory::{
        budget,
        current_address_space_handle,
    },
    self_test::close_test_address_space,
};

fn scratch_candidate(owner: usize) -> VAddr {
    let base = reserve_scratch(owner, 1).unwrap();
    release_scratch(owner, base, 1).unwrap();
    base
}
fn map_scratch_with(
    owner: usize,
    cap: MemoryObjectCap,
    writable: bool,
    invalidate: impl FnOnce(AddressSpaceId, VAddr, usize) -> Result<(), InvalidationError>,
) -> Result<VAddr, MemoryObjectError> {
    with_address_space_operation(owner, || map_any_with_lease(owner, cap, writable, invalidate))
}
fn map_fixed_with(
    owner: usize,
    cap: MemoryObjectCap,
    base: VAddr,
    invalidate: impl FnOnce(AddressSpaceId, VAddr, usize) -> Result<(), InvalidationError>,
) -> Result<(), MemoryObjectError> {
    with_address_space_operation(owner, || map_with_lease(owner, cap, base, true, invalidate))
}

pub(super) fn run(create: &mut impl FnMut(&str) -> usize) {
    preflight_rejection(create);
    rejected_prefix_still_invalidates(create);
    for &cause in causes() {
        for scratch in [false, true] {
            failed_invalidation(create, cause, scratch);
        }
    }
    crate::logln!(
        "[object mapping admission] 256 pin-free denials skipped maintenance and recovered \
         scratch; admitted success/rolled-back prefix still invalidate; per-call causes retain \
         exact mapping/backing pin and original charge without replay"
    );
}

fn preflight_rejection(create: &mut impl FnMut(&str) -> usize) {
    let owner = create("mapping preflight denial");
    let handle = current_address_space_handle(owner).unwrap();
    let cap = allocate(owner, 1).unwrap();
    let id = MEMORY_OBJECTS.lock().lookup(owner, cap).unwrap().object;
    let copy = pin_for_copy(owner, cap).unwrap();
    let candidate = scratch_candidate(owner);
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let charge = budget::used(handle);
    for _ in 0..128 {
        assert_eq!(
            map_scratch_with(owner, cap, true, |_, _, _| panic!(
                "preflight denial entered maintenance"
            )),
            Err(MemoryObjectError::LendingActive)
        );
        assert_eq!(
            map_fixed_with(owner, cap, VAddr::from(0x160000usize), |_, _, _| panic!(
                "preflight denial entered maintenance"
            )),
            Err(MemoryObjectError::LendingActive)
        );
        assert_eq!(scratch_candidate(owner), candidate);
        assert_eq!(MEMORY_OBJECTS.lock().objects[&id].retirement_pins, 0);
        assert!(!info(owner, cap).unwrap().mapped);
        assert_eq!(budget::used(handle), charge);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
    }
    unpin_copy(copy);
    let mut calls = 0;
    let mapped = map_scratch_with(owner, cap, true, |asid, base, pages| {
        calls += 1;
        assert!(MEMORY_OBJECTS.try_lock().is_some());
        assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
        assert_eq!(asid, owner);
        assert_eq!(base, candidate);
        assert_eq!(pages, 1);
        assert_eq!(MEMORY_OBJECTS.lock().objects[&id].retirement_pins, 1);
        crate::cpu::isa::memory::tlb::try_inval_range_user(asid, base, pages)
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(mapped, candidate);
    assert_eq!(MEMORY_OBJECTS.lock().objects[&id].retirement_pins, 0);
    unmap(owner, cap).unwrap();
    close_cap(owner, cap).unwrap();
    close_test_address_space(owner).unwrap();
    assert_eq!(budget::used(handle), budget::Amount::default());
}

fn rejected_prefix_still_invalidates(create: &mut impl FnMut(&str) -> usize) {
    let owner = create("mapping rejected prefix");
    let cap = allocate(owner, 2).unwrap();
    let foreign = allocate(owner, 1).unwrap();
    let id = MEMORY_OBJECTS.lock().lookup(owner, cap).unwrap().object;
    let base = VAddr::from(0x180000usize);
    map(owner, foreign, base + PAGE_SIZE, true).unwrap();
    let mut calls = 0;
    let result = map_fixed_with(owner, cap, base, |asid, address, pages| {
        calls += 1;
        assert_eq!(MEMORY_OBJECTS.lock().objects[&id].retirement_pins, 1);
        assert_eq!(pages, 2);
        crate::cpu::isa::memory::tlb::try_inval_range_user(asid, address, pages)
    });
    assert_eq!(result, Err(MemoryObjectError::MapFailed));
    assert_eq!(calls, 1, "rollback needs maintenance despite rejected mapping");
    assert_eq!(MEMORY_OBJECTS.lock().objects[&id].retirement_pins, 0);
    assert!(!info(owner, cap).unwrap().mapped);
    assert!(info(owner, foreign).unwrap().mapped);
    close_cap(owner, cap).unwrap();
    unmap(owner, foreign).unwrap();
    close_cap(owner, foreign).unwrap();
    close_test_address_space(owner).unwrap();
}

fn causes() -> &'static [InvalidationError] {
    #[cfg(target_arch = "x86_64")]
    {
        &[
            InvalidationError::Busy,
            InvalidationError::InterruptsMasked,
            InvalidationError::Exhausted,
            InvalidationError::Delivery(2),
            InvalidationError::TimedOut {
                epoch: 91,
                first_missing: Some(2),
            },
        ]
    }
    #[cfg(target_arch = "aarch64")]
    {
        &[InvalidationError::AddressSpaceMissing]
    }
}

fn failed_invalidation(
    create: &mut impl FnMut(&str) -> usize,
    cause: InvalidationError,
    scratch: bool,
) {
    let owner = create("mapping unconfirmed invalidation");
    let handle = current_address_space_handle(owner).unwrap();
    let cap = allocate(owner, 1).unwrap();
    let id = MEMORY_OBJECTS.lock().lookup(owner, cap).unwrap().object;
    let candidate = scratch_candidate(owner);
    let charge = budget::used(handle);
    let mut calls = 0;
    let fail = |asid, _, pages| {
        calls += 1;
        assert_eq!(asid, owner);
        assert_eq!(pages, 1);
        assert!(MEMORY_OBJECTS.try_lock().is_some());
        assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
        Err(cause)
    };
    let result = if scratch {
        map_scratch_with(owner, cap, true, fail).map(|_| ())
    } else {
        map_fixed_with(owner, cap, VAddr::from(0x1a0000usize), fail)
    };
    assert_eq!(result, Err(MemoryObjectError::InvalidationFailed(cause)));
    assert_eq!(calls, 1);
    assert!(info(owner, cap).unwrap().mapped);
    assert_eq!(MEMORY_OBJECTS.lock().objects[&id].retirement_pins, 1);
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert_eq!(close_cap(owner, cap), Err(MemoryObjectError::LendingActive));
    assert_eq!(unmap(owner, cap), Err(MemoryObjectError::LendingActive));
    assert_eq!(
        map_scratch_with(owner, cap, true, |_, _, _| panic!("uncertain mapping was replayed")),
        Err(MemoryObjectError::LendingActive)
    );
    if scratch {
        assert_ne!(scratch_candidate(owner), candidate);
    }
    assert_eq!(budget::used(handle), charge);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
    // Keep the actual mapping, pin, root and original charge. No fixture-only
    // adoption or synthetic completion may recover this uncertain operation.
}
