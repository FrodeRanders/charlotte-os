//! Serialized actual-buffer pressure, without quota resizing or synthetic refunds.
use super::*;
use crate::memory::{
    self,
    AddressSpaceHandle,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn root() -> AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}
fn trusted_root() -> AddressSpaceHandle {
    let handle = root();
    memory::budget::mark_platform(handle);
    handle
}
fn available() {
    assert!(MEMORY_OBJECTS.try_lock().is_some());
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some());
    assert!(memory::usage::principal_history_available());
}
fn pressure(handle: AddressSpaceHandle, pages: usize, owners: &mut Vec<Preparing>, bound: usize) {
    loop {
        match Preparing::new(handle, pages) {
            Ok(owner) => {
                assert!(owners.len() < bound);
                owners.push(owner);
            }
            Err(MemoryObjectError::ResourceLimit) => return,
            Err(error) => panic!("snapshot pressure allocation rejected before quota: {error:?}"),
        }
    }
}
fn finish_all(mut owners: Vec<Preparing>) {
    while let Some(owner) = owners.pop() {
        owner.finish();
    }
}

pub(crate) fn run() {
    allocation_rejection();
    release_order_and_classification();
    count_pressure();
    byte_pressure();
    principal_coverage();
    exact_capacity();
    crate::logln!(
        "[snapshot buffer admission] 64 charged allocation rejections; 64 \
         buffer-free-before-refund observations; real ordinary/total count and byte exhaustion, \
         three pin/map rejection paths, empty escrow, trusted reserve, promotion/root reuse and \
         exact baseline recovery passed; no quota reset"
    );
}

fn signed_root(name: &[u8]) -> (AddressSpaceHandle, u64) {
    let principal = charlotte_launch::artifact_principal_id(name);
    let space = memory::AddressSpace::try_new_principal_user(
        false,
        memory::prepare_principal_backing(principal, false).unwrap(),
    )
    .unwrap();
    let handle = memory::register_user_address_space(space).unwrap();
    memory::register_domain_authority(handle, principal, 0).unwrap();
    (handle, principal)
}

fn exact_capacity() {
    let (handle, principal) = signed_root(b"snapshot-capacity-rejection");
    let before = admitted();
    for over_capacity in [false, true] {
        let result = Preparing::with_factory(
            handle,
            4,
            |_| Ok(()),
            |frames, pages| {
                frames.try_reserve_exact(pages + usize::from(over_capacity)).unwrap();
                if !over_capacity {
                    frames.push(PAddr::try_from(0usize).unwrap());
                }
                Ok(())
            },
        );
        assert_eq!(result.err(), Some(MemoryObjectError::FrameAllocFailed));
        assert_eq!(admitted(), before);
        assert_eq!(memory::kernel_buffers::test_used(principal), 0);
    }
    memory::close_user_address_space_handle(handle).unwrap();
}

fn principal_coverage() {
    use memory::kernel_buffers::{
        test_limit,
        test_used,
    };
    let before = admitted();
    let (first, principal) = signed_root(b"snapshot-frame-principal");
    let (sibling, same) = signed_root(b"snapshot-frame-principal");
    let (healthy, other) = signed_root(b"snapshot-frame-healthy");
    assert_eq!(principal, same);
    test_limit(principal, 16);
    let cap = allocate(first.id(), 1).unwrap();
    let copy = pin_for_copy(first.id(), cap).unwrap();
    let prepared = Preparing::new(sibling, 1).unwrap();
    assert_eq!(test_used(principal), 16);
    let full = admitted();
    for _ in 0..64 {
        denied(first, cap, 1);
    }
    assert_eq!(test_used(principal), 16);
    let healthy_snapshot = Preparing::new(healthy, 1).unwrap();
    assert_eq!(test_used(other), 8);
    memory::budget::mark_platform(first);
    assert_eq!(admitted().1, [full.1[0] + 8, full.1[1] + 1]);
    assert!(Preparing::new(first, 1).is_err());
    memory::close_user_address_space_handle(first).unwrap();
    assert_eq!(test_used(principal), 16);
    let (replacement, replacement_principal) = signed_root(b"snapshot-frame-healthy");
    assert_eq!(replacement.id(), first.id());
    assert_ne!(replacement, first);
    assert_eq!(replacement_principal, other);
    unpin_copy(copy);
    assert_eq!(test_used(principal), 8);
    prepared.finish();
    assert_eq!(test_used(principal), 0);
    assert_eq!(test_used(other), 8);
    let sibling_cap = allocate(sibling.id(), 1).unwrap();
    let dma = pin_for_dma(sibling.id(), sibling_cap, true, false, false).unwrap();
    assert_eq!(test_used(principal), 8);
    unpin_dma(dma); // Known unused pin: no hardware mapping was published.
    assert_eq!(test_used(principal), 0);
    let mut orphan = Preparing::new(sibling, 1).unwrap();
    let snapshot = core::mem::take(&mut orphan.0);
    orphan.finish();
    let charged = admitted();
    snapshot.finish_with(|| {
        available();
        assert_eq!(admitted(), charged);
        assert_eq!(test_used(principal), 8, "principal refund preceded buffer free");
    });
    assert_eq!(test_used(principal), 0);
    for handle in [sibling, healthy, replacement] {
        memory::close_user_address_space_handle(handle).unwrap();
    }
    assert_eq!(test_used(other), 8);
    healthy_snapshot.finish();
    assert_eq!(test_used(other), 0);
    assert_eq!(admitted(), before);
    // Complete snapshot abandonment retains both counters under their guards.
    let (retained, retained_principal) = signed_root(b"snapshot-frame-retained");
    let snapshot = Preparing::new(retained, 1).unwrap();
    let charged = admitted();
    memory::usage::with_principal_history_locked(|| {
        let _heap = PRIMARY_ALLOCATOR.lock();
        admission::with_locked(|| drop(snapshot));
    });
    assert_eq!(admitted(), charged);
    assert_eq!(test_used(retained_principal), 8);
    memory::close_user_address_space_handle(retained).unwrap();
    assert_eq!(test_used(retained_principal), 8);
    crate::logln!(
        "[snapshot principal frames] shared-root copy/DMA/map denials, original class, \
         independent sponsor, retained old-principal refund after close/reuse, free-before-refund \
         and guarded abandonment passed"
    );
}

fn allocation_rejection() {
    let (handle, principal) = signed_root(b"snapshot-allocation-rejection");
    let before = admitted();
    for _ in 0..64 {
        let result = Preparing::with_factory(
            handle,
            35,
            |_| Ok(()),
            |_, pages| {
                available();
                assert_eq!(pages, 35);
                assert_eq!(admitted().0, [before.0[0] + 35 * 8, before.0[1] + 1]);
                assert_eq!(memory::kernel_buffers::test_used(principal), 35 * 8);
                Err(MemoryObjectError::FrameAllocFailed)
            },
        );
        assert_eq!(result.err(), Some(MemoryObjectError::FrameAllocFailed));
        assert_eq!(admitted(), before);
        assert_eq!(memory::kernel_buffers::test_used(principal), 0);
    }
    for pages in [0, usize::MAX] {
        assert_eq!(Preparing::new(handle, pages).err(), Some(MemoryObjectError::FrameAllocFailed));
        assert_eq!(admitted(), before);
    }
    memory::close_user_address_space_handle(handle).unwrap();
}

fn release_order_and_classification() {
    let handle = root();
    let before = admitted();
    for _ in 0..64 {
        let mut owner = Preparing::new(handle, 35).unwrap();
        let snapshot = core::mem::take(&mut owner.0);
        owner.finish();
        let charged = admitted();
        snapshot.finish_with(|| {
            available();
            assert_eq!(admitted(), charged, "buffer free must precede admission refund");
        });
        assert_eq!(admitted(), before);
    }
    let owner = Preparing::with(handle, 35, |stage| {
        if stage == 0 {
            memory::budget::mark_platform(handle);
        }
        Ok(())
    })
    .unwrap();
    let charged = admitted();
    assert_eq!(charged.1, [before.1[0] + 35 * 8, before.1[1] + 1]);
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(admitted(), charged);
    let fresh = root();
    assert_eq!(fresh.id(), handle.id());
    assert_ne!(fresh, handle);
    owner.finish();
    assert_eq!(admitted(), before);
    memory::close_user_address_space_handle(fresh).unwrap();
}

fn denied(handle: AddressSpaceHandle, cap: MemoryObjectCap, copies: usize) {
    let before = admitted();
    assert_eq!(pin_for_copy(handle.id(), cap).err(), Some(MemoryObjectError::ResourceLimit));
    assert_eq!(
        pin_for_dma(handle.id(), cap, true, false, false).err(),
        Some(MemoryObjectError::ResourceLimit)
    );
    assert_eq!(
        map(handle.id(), cap, VAddr::from(0x2a0000usize), false),
        Err(MemoryObjectError::ResourceLimit)
    );
    let registry = MEMORY_OBJECTS.lock();
    let id = registry.lookup(handle.id(), cap).unwrap().object;
    let object = &registry.objects[&id];
    assert_eq!(object.copy_pins, copies);
    assert_eq!((object.dma_pins, object.retirement_pins), (0, 0));
    assert!(object.mappings.first_key_value().is_none());
    drop(registry);
    assert_eq!(admitted(), before);
}

fn count_pressure() {
    let before = admitted();
    let handle = root();
    let trusted = trusted_root();
    let cap = allocate(handle.id(), 1).unwrap();
    let movable = allocate(handle.id(), 1).unwrap();
    let old_pin = pin_for_copy(handle.id(), cap).unwrap();
    let old_object = old_pin.object_id();
    let mut ordinary = Vec::with_capacity(admission::TOTAL_LIMIT[1] as usize);
    pressure(handle, 1, &mut ordinary, admission::TOTAL_LIMIT[1] as usize);
    assert_eq!(admitted().1[1], admission::ORDINARY_LIMIT[1]);
    for _ in 0..64 {
        assert_eq!(
            Preparing::with_factory(
                handle,
                1,
                |_| Ok(()),
                |_, _| panic!("quota denial entered allocator")
            )
            .err(),
            Some(MemoryObjectError::ResourceLimit)
        );
    }
    denied(handle, cap, 1);
    let full_ordinary = admitted();
    // Empty source-escrow pins have no buffer allocation and remain usable.
    drop(prepare_move(handle.id(), movable, trusted.id()).unwrap());
    assert_eq!(admitted(), full_ordinary);
    close_cap(handle.id(), movable).unwrap();
    let trusted_cap = allocate(trusted.id(), 1).unwrap();
    let pin = pin_for_copy(trusted.id(), trusted_cap).unwrap();
    assert_eq!(admitted().1, full_ordinary.1);
    unpin_copy(pin);
    assert_eq!(admitted(), full_ordinary);
    let mut reserve = Vec::with_capacity(admission::TOTAL_LIMIT[1] as usize);
    pressure(trusted, 1, &mut reserve, admission::TOTAL_LIMIT[1] as usize);
    let full = admitted();
    assert_eq!(full.0[1], admission::TOTAL_LIMIT[1]);
    memory::budget::mark_platform(handle);
    assert_eq!(admitted(), full);
    // An unused backing pin needs no new snapshot admission to close. The
    // unmapped source root can retire, but original pin/buffer charges stay live.
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(admitted(), full);
    unpin_copy(old_pin);
    assert_eq!(admitted(), ([full.0[0] - 8, full.0[1] - 1], [full.1[0] - 8, full.1[1] - 1]));
    let fresh = root();
    assert_eq!(fresh.id(), handle.id());
    assert_ne!(fresh, handle);
    let fresh_cap = allocate(fresh.id(), 1).unwrap();
    let pin = pin_for_copy(fresh.id(), fresh_cap).unwrap();
    assert_ne!(pin.object_id(), old_object);
    assert_eq!(admitted(), full);
    unpin_copy(pin);
    close_cap(fresh.id(), fresh_cap).unwrap();
    memory::close_user_address_space_handle(fresh).unwrap();
    let counts = (ordinary.len(), reserve.len());
    finish_all(reserve);
    finish_all(ordinary);
    close_cap(trusted.id(), trusted_cap).unwrap();
    memory::close_user_address_space_handle(trusted).unwrap();
    assert_eq!(admitted(), before);
    crate::logln!(
        "[snapshot count pressure] {} ordinary + {} reserve actual buffers; {}/{} limits, three \
         adapter denials, empty escrow and exact old/new-generation unpin refund passed",
        counts.0,
        counts.1,
        admission::ORDINARY_LIMIT[1],
        admission::TOTAL_LIMIT[1]
    );
}

fn byte_pressure() {
    let before = admitted();
    let handle = root();
    let trusted = trusted_root();
    let cap = allocate(handle.id(), 1).unwrap();
    let mut ordinary = Vec::with_capacity(
        (admission::TOTAL_LIMIT[0] / (MAX_MEMORY_OBJECT_PAGES * 8) as u64 + 2) as usize,
    );
    pressure(
        handle,
        MAX_MEMORY_OBJECT_PAGES,
        &mut ordinary,
        (admission::TOTAL_LIMIT[0] / (MAX_MEMORY_OBJECT_PAGES * 8) as u64 + 2) as usize,
    );
    let remainder = admission::ORDINARY_LIMIT[0] - admitted().1[0];
    if remainder != 0 {
        ordinary.push(Preparing::new(handle, remainder as usize / 8).unwrap());
    }
    assert_eq!(admitted().1[0], admission::ORDINARY_LIMIT[0]);
    assert!(admitted().1[1] < admission::ORDINARY_LIMIT[1]);
    denied(handle, cap, 0);
    let mut reserve = Vec::with_capacity(
        (admission::TOTAL_LIMIT[0] / (MAX_MEMORY_OBJECT_PAGES * 8) as u64 + 2) as usize,
    );
    pressure(
        trusted,
        MAX_MEMORY_OBJECT_PAGES,
        &mut reserve,
        (admission::TOTAL_LIMIT[0] / (MAX_MEMORY_OBJECT_PAGES * 8) as u64 + 2) as usize,
    );
    let remainder = admission::TOTAL_LIMIT[0] - admitted().0[0];
    if remainder != 0 {
        reserve.push(Preparing::new(trusted, remainder as usize / 8).unwrap());
    }
    assert_eq!(admitted().0[0], admission::TOTAL_LIMIT[0]);
    assert!(admitted().0[1] < admission::TOTAL_LIMIT[1]);
    let counts = (ordinary.len(), reserve.len());
    finish_all(reserve);
    finish_all(ordinary);
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
    memory::close_user_address_space_handle(trusted).unwrap();
    assert_eq!(admitted(), before);
    crate::logln!(
        "[snapshot byte pressure] {} ordinary + {} reserve actual buffers, up to 128 KiB each; \
         exact {}/{} byte limits before count limits and post-deallocation baseline recovery \
         passed",
        counts.0,
        counts.1,
        admission::ORDINARY_LIMIT[0],
        admission::TOTAL_LIMIT[0]
    );
}
