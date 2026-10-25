//! Serialized real-node pressure. Never resize, reset or synthesize refunds.
use alloc::vec::Vec;
use core::alloc::AllocError;

use super::*;

pub(super) fn run() {
    allocation_failure::<(usize, usize)>();
    allocation_failure::<(AddressSpaceId, Namespace)>();
    captured_classification();
    pressure();
    byte_pressure();
    crate::logln!(
        "[scratch admission] 128 allocation rejections; real ordinary/total byte and node \
         exhaustion, partial-quota rollback, trusted reserve, captured classification and \
         exact-generation close/reuse passed; no budget reset"
    );
}

fn allocation_failure<T>() {
    let before = admission::snapshot();
    for _ in 0..64 {
        assert!(
            admission::prepare_with::<T>(false, |_| {
                available();
                assert_eq!(admission::snapshot().0[1], before.0[1] + 1);
                Err(AllocError)
            })
            .is_err()
        );
        assert_eq!(admission::snapshot(), before);
    }
}

fn captured_classification() {
    let handle = root();
    let before = admission::snapshot();
    let owner = Preparing::with(handle, |stage| {
        if stage == 1 {
            memory::budget::mark_platform(handle);
        }
        Ok(())
    })
    .unwrap();
    let bytes = (ExtentStorage::allocation_bytes() + Storage::allocation_bytes()) as u64;
    assert_eq!(
        admission::snapshot(),
        ([before.0[0] + bytes, before.0[1] + 2], [before.1[0] + bytes, before.1[1] + 2],)
    );
    owner.finish();
    assert_eq!(admission::snapshot(), before);
    memory::close_user_address_space_handle(handle).unwrap();
}

fn denied(handle: AddressSpaceHandle) {
    let before = admission::snapshot();
    assert_eq!(reserve_scratch(handle.id(), 1), Err(MemoryObjectError::ResourceLimit));
    assert_eq!(admission::snapshot(), before);
    assert_eq!(SCRATCH_WINDOWS.lock().get(&handle.id()).unwrap().window.extent_count(), 1);
}

fn pressure() {
    let before = admission::snapshot();
    let handle = root();
    reserve_scratch(handle.id(), 1).unwrap();
    let mut ordinary: Vec<ExtentStorage> = Vec::with_capacity(admission::TOTAL_LIMIT[1] as usize);
    while let Ok(node) = admission::prepare(false) {
        ordinary.push(node);
        assert!(ordinary.len() <= admission::TOTAL_LIMIT[1] as usize);
    }
    assert_eq!(admission::snapshot().1[1], admission::ORDINARY_LIMIT[1]);
    for _ in 0..64 {
        assert!(
            admission::prepare_with::<(usize, usize)>(false, |_| {
                panic!("scratch quota rejection entered allocator")
            })
            .is_err()
        );
    }
    denied(handle);
    // Only one available slot cannot publish the two-node preparation. The
    // first actual allocation must return with its original charge on rejection.
    drop(ordinary.pop().unwrap());
    denied(handle);
    ordinary.push(admission::prepare(false).unwrap());
    let full_ordinary = admission::snapshot();
    memory::budget::mark_platform(handle);
    assert_eq!(admission::snapshot(), full_ordinary);
    let trusted =
        memory::register_user_address_space(memory::AddressSpace::try_new_platform_user().unwrap())
            .unwrap();
    // Translation preparation and the published memory sponsor are separate
    // trust boundaries. Install the fixture's trusted sponsor explicitly.
    assert_ne!(memory::budget::platform_identity(trusted.id()), Some(trusted));
    memory::budget::mark_platform(trusted);
    let base = reserve_scratch(trusted.id(), 1).unwrap();
    assert_eq!(admission::snapshot().1, full_ordinary.1);
    assert_eq!(admission::snapshot().0[1], full_ordinary.0[1] + 2);
    release_scratch(trusted.id(), base, 1).unwrap();
    memory::close_user_address_space_handle(trusted).unwrap();
    assert_eq!(admission::snapshot(), full_ordinary);

    let mut reserve: Vec<ExtentStorage> = Vec::with_capacity(admission::TOTAL_LIMIT[1] as usize);
    while let Ok(node) = admission::prepare(true) {
        reserve.push(node);
        assert!(reserve.len() <= admission::TOTAL_LIMIT[1] as usize);
    }
    let full = admission::snapshot();
    assert_eq!(full.0[1], admission::TOTAL_LIMIT[1]);
    assert!(
        admission::prepare_with::<(usize, usize)>(true, |_| {
            panic!("scratch total rejection entered allocator")
        })
        .is_err()
    );
    memory::close_user_address_space_handle(handle).unwrap();
    let freed_bytes = (ExtentStorage::allocation_bytes() + Storage::allocation_bytes()) as u64;
    assert_eq!(
        admission::snapshot(),
        ([full.0[0] - freed_bytes, full.0[1] - 2], [full.1[0] - freed_bytes, full.1[1] - 2],)
    );
    let successor = root();
    assert_eq!(successor.id(), handle.id());
    assert_ne!(successor, handle);
    reserve_scratch(successor.id(), 1).unwrap();
    assert_eq!(admission::snapshot(), full);
    memory::close_user_address_space_handle(successor).unwrap();
    let counts = (ordinary.len(), reserve.len());
    drop(reserve);
    drop(ordinary);
    assert_eq!(admission::snapshot(), before);
    crate::logln!(
        "[scratch node pressure] {} ordinary + {} reserved actual extent nodes; {}/{} count \
         ceilings, one-slot rollback, trusted publication and close at total exhaustion passed",
        counts.0,
        counts.1,
        admission::ORDINARY_LIMIT[1],
        admission::TOTAL_LIMIT[1]
    );
}

fn byte_pressure() {
    let before = admission::snapshot();
    let bytes = Storage::allocation_bytes() as u64;
    let mut ordinary: Vec<Storage> = Vec::with_capacity(admission::TOTAL_LIMIT[1] as usize);
    while let Ok(node) = admission::prepare(false) {
        ordinary.push(node);
        assert!(ordinary.len() <= admission::TOTAL_LIMIT[1] as usize);
    }
    let used = admission::snapshot();
    assert!(used.1[0] + bytes > admission::ORDINARY_LIMIT[0]);
    assert!(used.1[1] < admission::ORDINARY_LIMIT[1]);
    let mut reserve: Vec<Storage> = Vec::with_capacity(admission::TOTAL_LIMIT[1] as usize);
    while let Ok(node) = admission::prepare(true) {
        reserve.push(node);
        assert!(reserve.len() <= admission::TOTAL_LIMIT[1] as usize);
    }
    let used = admission::snapshot();
    assert!(used.0[0] + bytes > admission::TOTAL_LIMIT[0]);
    assert!(used.0[1] < admission::TOTAL_LIMIT[1]);
    let counts = (ordinary.len(), reserve.len());
    drop(reserve);
    drop(ordinary);
    assert_eq!(admission::snapshot(), before);
    crate::logln!(
        "[scratch byte pressure] {} ordinary + {} reserved actual namespace nodes, {} bytes each; \
         {}/{} byte ceilings reject before count limits and return exact baseline after \
         deallocation",
        counts.0,
        counts.1,
        bytes,
        admission::ORDINARY_LIMIT[0],
        admission::TOTAL_LIMIT[0]
    );
}
