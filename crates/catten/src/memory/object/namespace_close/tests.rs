//! Raw kernel ABI fixtures for exact namespace cleanup ownership. Failed
//! receipts retain their real roots, mappings/backing and sponsorship; no
//! test-only reclamation path reconstructs abandoned owners.

use super::*;
use crate::{
    memory::{
        AddressSpaceCloseError,
        backing_budget,
        budget,
        retirement::{
            CloseProgress,
            RetirementProgress,
        },
    },
    service::loader,
};

fn unlocked() {
    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
    assert!(MEMORY_OBJECTS.try_lock().is_some());
    assert!(SCRATCH_WINDOWS.try_lock().is_some());
}

fn barrier(asid: usize, base: VAddr, pages: usize) -> bool {
    unlocked();
    crate::cpu::isa::memory::tlb::inval_range_user(asid, base, pages);
    true
}

pub(crate) fn run() {
    success();
    unmapped_revocation_peer();
    preparation_rejection();
    for failure in [Failure::Detach, Failure::Barrier, Failure::Scratch, Failure::Abandon] {
        failure_case(failure);
    }
    crate::logln!(
        "[memory namespace] mapped peers retained outside lifecycle, closing peers, unmapped \
         revocation fence, preparation rollback and four failure/Drop quarantines passed"
    );
}

fn success() {
    let owner = loader::create_user_address_space_handle();
    let first = loader::create_user_address_space_handle();
    let second = loader::create_user_address_space_handle();
    let pages = RETIREMENT_FRAME_BATCH * 2 + 3;
    let cap = allocate(owner.id(), pages).unwrap();
    let read = lend_read(owner.id(), cap, first.id()).unwrap();
    let other = lend_read(owner.id(), cap, second.id()).unwrap();
    map_any(owner.id(), cap, false).unwrap();
    let first_base = map_any(first.id(), read, false).unwrap();
    map(second.id(), other, VAddr::from(0x3000_0000usize), false).unwrap();
    let mut peer = Some(ClosingAddressSpace::begin_ready(first).unwrap());
    assert!(peer.as_mut().unwrap().start_cleanup().unwrap());
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let mut batches = 0;
    let retired = ClosingAddressSpace::begin_ready(owner)
        .unwrap()
        .prepare_with_cleanup(LoanRevocation::finish, |receipt| {
            unlocked();
            assert_eq!(info(owner.id(), cap).unwrap().pages, pages);
            assert_eq!(budget::used(owner).pages, pages as u64);
            assert_eq!(
                crate::memory::close_user_address_space_handle(second),
                Err(AddressSpaceCloseError::OperationsInFlight)
            );
            let RetirementProgress::Pending(pending) =
                peer.take().unwrap().prepare_retirement().unwrap()
            else {
                panic!("mapped peer closed before its invalidation")
            };
            peer = Some(pending);
            receipt.finish_with(
                |asid, base, frames| {
                    unlocked();
                    batches += 1;
                    unmap_pages(asid, base, frames)
                },
                |asid, base, pages| {
                    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
                    barrier(asid, base, pages)
                },
                |asid, base, pages| {
                    unlocked();
                    release_scratch(asid, base, pages)
                },
            )
        })
        .unwrap();
    let RetirementProgress::Ready(retired) = retired else {
        panic!("ready namespace remained pending")
    };
    assert_eq!(batches, 9);
    assert_eq!(budget::used(owner), budget::Amount::default());
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before + pages);
    assert_eq!(info(first.id(), read), Err(MemoryObjectError::UnknownCapability));
    let reused = reserve_scratch(first.id(), pages).unwrap();
    assert_eq!(reused, first_base);
    release_scratch(first.id(), reused, pages).unwrap();
    retired.release().unwrap();
    assert!(matches!(peer.take().unwrap().poll().unwrap(), CloseProgress::Complete));
    crate::memory::close_user_address_space_handle(second).unwrap();
}

fn unmapped_revocation_peer() {
    let owner = loader::create_user_address_space_handle();
    let target = loader::create_user_address_space_handle();
    let third = loader::create_user_address_space_handle();
    let cap = allocate(owner.id(), 1).unwrap();
    let loan = lend_read(owner.id(), cap, target.id()).unwrap();
    let remaining = lend_read(owner.id(), cap, third.id()).unwrap();
    let revocation = LoanRevocation::prepare(owner.id(), cap, target.id(), loan).unwrap();
    // Its prior state owns the third reader even though that reader has no
    // mapping. Namespace removal must wait before erasing the reader's cap.
    let closing = ClosingAddressSpace::begin_ready(third).unwrap();
    let CloseProgress::Pending(closing) = closing.poll().unwrap() else {
        panic!("unmapped revocation peer retired before prior state returned")
    };
    assert!(MEMORY_OBJECTS.lock().caps[&third.id()].caps.contains_key(&remaining));
    assert_eq!(crate::memory::current_address_space_handle(third.id()), Some(third));
    revocation.finish().unwrap();
    assert!(matches!(closing.poll().unwrap(), CloseProgress::Complete));
    assert!(!info(owner.id(), cap).unwrap().lent);
    close_cap(owner.id(), cap).unwrap();
    crate::memory::close_user_address_space_handle(owner).unwrap();
    crate::memory::close_user_address_space_handle(target).unwrap();
}

fn preparation_rejection() {
    let owner = loader::create_user_address_space_handle();
    let first = loader::create_user_address_space_handle();
    let old = loader::create_user_address_space_handle();
    crate::memory::close_user_address_space_handle(old).unwrap();
    let fresh = loader::create_user_address_space_handle();
    assert_eq!(old.id(), fresh.id());
    assert_ne!(old, fresh);
    let cap = allocate(owner.id(), 1).unwrap();
    let one = lend_read(owner.id(), cap, first.id()).unwrap();
    let two = lend_read(owner.id(), cap, fresh.id()).unwrap();
    map_any(first.id(), one, false).unwrap();
    let base = map_any(fresh.id(), two, false).unwrap();
    let id = MEMORY_OBJECTS.lock().lookup(owner.id(), cap).unwrap().object;
    // Corrupt only the captured generation at this private test boundary.
    // The real successor mapping must never be walked by stale cleanup.
    MEMORY_OBJECTS
        .lock()
        .objects
        .get_mut(&id)
        .unwrap()
        .mappings
        .get_mut(&fresh.id())
        .unwrap()
        .state
        .address_space = old;
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert_eq!(
        ClosingAddressSpace::begin_ready(owner).unwrap().prepare_retirement().err(),
        Some(AddressSpaceCloseError::MemoryCleanupFailed)
    );
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
    let registry = MEMORY_OBJECTS.lock();
    let object = &registry.objects[&id];
    assert_eq!(object.retirement_pins, 0);
    assert!(!object.destroy_when_unpinned);
    assert!(object.mappings.values().all(|mapping| mapping.cleanup_lease.is_none()));
    let frame = object.frames[0];
    drop(registry);
    assert_eq!(
        ADDRESS_SPACE_TABLE.lock().get_mut(fresh.id()).unwrap().translate_address(base).unwrap(),
        frame
    );
    // Earlier peer admission was explicitly returned on preparation rejection.
    crate::memory::close_user_address_space_handle(first).unwrap();
    AddressSpaceOperation::acquire(fresh).unwrap().release().unwrap();
    assert_eq!(crate::memory::current_address_space_handle(owner.id()), Some(owner));
    crate::logln!(
        "[memory namespace] stale mapped peer rejected; owner/charged object and successor \
         mapping retained"
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Failure {
    Detach,
    Barrier,
    Scratch,
    Abandon,
}

fn failure_case(failure: Failure) {
    let owner = loader::create_user_address_space_handle();
    let peer = loader::create_user_address_space_handle();
    assert!(crate::memory::commit_user_heap_page_handle(owner, charlotte_launch::HEAP_VADDR));
    let heap = backing_budget::test_used_pages(backing_budget::Kind::Heap);
    let cap = allocate(owner.id(), 2).unwrap();
    let loan = lend_read(owner.id(), cap, peer.id()).unwrap();
    let base = map_any(peer.id(), loan, false).unwrap();
    let id = MEMORY_OBJECTS.lock().lookup(owner.id(), cap).unwrap().object;
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert_eq!(
        ClosingAddressSpace::begin_ready(owner)
            .unwrap()
            .prepare_with_cleanup(LoanRevocation::finish, |receipt| {
                unlocked();
                if failure == Failure::Abandon {
                    drop(receipt);
                    return Err(MemoryObjectError::UnmapFailed);
                }
                receipt.finish_with(
                    |asid, base, frames| {
                        unlocked();
                        if failure == Failure::Detach {
                            unmap_pages(asid, base, &frames[..1])?;
                            return Err(MemoryObjectError::UnmapFailed);
                        }
                        unmap_pages(asid, base, frames)
                    },
                    |asid, base, pages| {
                        if failure == Failure::Barrier {
                            unlocked();
                            false
                        } else {
                            barrier(asid, base, pages)
                        }
                    },
                    |asid, base, pages| {
                        unlocked();
                        if failure == Failure::Scratch {
                            Err(MemoryObjectError::OutOfScratch)
                        } else {
                            release_scratch(asid, base, pages)
                        }
                    },
                )
            })
            .err(),
        Some(AddressSpaceCloseError::MemoryCleanupFailed)
    );
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
    assert_eq!(budget::used(owner).pages, 2);
    assert_eq!(backing_budget::test_used_pages(backing_budget::Kind::Heap), heap);
    assert_eq!(MEMORY_OBJECTS.lock().objects[&id].retirement_pins, 1);
    assert_eq!(crate::memory::current_address_space_handle(owner.id()), Some(owner));
    assert_eq!(crate::memory::current_address_space_handle(peer.id()), Some(peer));
    assert_eq!(
        crate::memory::close_user_address_space_handle(owner),
        Err(AddressSpaceCloseError::CloseInProgress)
    );
    assert_eq!(
        crate::memory::close_user_address_space_handle(peer),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    let temporary = reserve_scratch(peer.id(), 2).unwrap();
    assert_ne!(temporary, base);
    release_scratch(peer.id(), temporary, 2).unwrap();
}

/// Production invalidation with secondary LPs online; the roots have no
/// application threads and this does not inject failed recipient delivery.
pub(crate) fn run_runtime() {
    let owner = loader::create_user_address_space_handle();
    let peer = loader::create_user_address_space_handle();
    let cap = allocate(owner.id(), RETIREMENT_FRAME_BATCH + 1).unwrap();
    map_any(owner.id(), cap, false).unwrap();
    let loan = lend_read(owner.id(), cap, peer.id()).unwrap();
    map_any(peer.id(), loan, false).unwrap();
    crate::memory::close_user_address_space_handle(owner).unwrap();
    assert_eq!(info(peer.id(), loan), Err(MemoryObjectError::UnknownCapability));
    crate::memory::close_user_address_space_handle(peer).unwrap();
    crate::logln!(
        "[memory namespace runtime] local/peer mapped backing retired with secondary LPs online"
    );
}
