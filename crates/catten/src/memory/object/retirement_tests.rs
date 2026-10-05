//! Single-mutator boot fixtures at the raw kernel ownership boundary. Fault
//! adapters exercise the same detach/finish helpers as runtime teardown.
//! Quarantine probes intentionally reserve seven data pages for the guest's
//! lifetime; there is no test-only re-adoption/release escape hatch.

use super::*;
use crate::{
    memory::{
        budget,
        current_address_space_handle,
    },
    self_test::close_test_address_space,
};

fn amount(pages: u64) -> budget::Amount {
    budget::Amount {
        pages,
        objects: 1,
    }
}

fn object_id(asid: usize, cap: MemoryObjectCap) -> MemoryObjectId {
    MEMORY_OBJECTS.lock().lookup(asid, cap).unwrap().object
}

fn detach(id: MemoryObjectId, closing: usize) -> RetiredObjectMappings {
    let receipt = RetiredObjectMappings::prepare(MEMORY_OBJECTS.lock(), id, closing);
    receipt.detach_with(checked_unmap)
}

fn checked_unmap(asid: usize, base: VAddr, frames: &[PAddr]) -> Result<(), MemoryObjectError> {
    assert!(MEMORY_OBJECTS.try_lock().is_some(), "registry held during table detach");
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some(), "table held before detach");
    unmap_pages(asid, base, frames)
}

fn invalidate(asid: usize, base: VAddr, pages: usize) -> bool {
    assert!(MEMORY_OBJECTS.try_lock().is_some(), "registry held during invalidation");
    crate::cpu::isa::memory::tlb::inval_range_user(asid, base, pages);
    true
}

pub(crate) fn run(mut create: impl FnMut(&str) -> usize) {
    test_batched_detach(&mut create);
    test_scratch_completion_failure(&mut create);
    test_last_unpin(&mut create);
    test_borrower_fence(&mut create);
    test_failed_detach(&mut create);
    test_failed_barrier_and_abandonment(&mut create);
    test_failed_map_cleanup(&mut create);
    crate::logln!(
        "[object retirement] bounded lock-separated batches, last-unpin fence, borrower \
         authority, scratch rejection, partial detach, failed barrier, Drop quarantine and \
         foreign-leaf preservation passed (seven reserved data pages)"
    );
}

fn test_scratch_completion_failure(create: &mut impl FnMut(&str) -> usize) {
    let owner = create("retirement scratch failure owner");
    let first = create("retirement scratch success borrower");
    let second = create("retirement scratch failure borrower");
    let handle = current_address_space_handle(owner).unwrap();
    let cap = allocate(owner, 1).unwrap();
    let id = object_id(owner, cap);
    let first_loan = lend_read(owner, cap, first).unwrap();
    let second_loan = lend_read(owner, cap, second).unwrap();
    let first_base = map_any(first, first_loan, false).unwrap();
    let second_base = map_any(second, second_loan, false).unwrap();
    let mut copy = Some(pin_for_copy(owner, cap).unwrap());
    let mut dma = Some(pin_for_dma(second, second_loan, true, false, false).unwrap());
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    {
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let receipt = detach(id, owner);
        let mut releases = 0;
        let barriers = core::cell::Cell::new(0);
        receipt.finish_with_scratch(
            owner,
            |asid, base, pages| {
                if let Some(pin) = copy.take() {
                    unpin_copy(pin);
                }
                if let Some(pin) = dma.take() {
                    unpin_dma(pin);
                }
                assert_eq!(budget::used(handle), amount(1));
                assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
                barriers.set(barriers.get() + 1);
                invalidate(asid, base, pages)
            },
            |asid, base, pages| {
                assert!(MEMORY_OBJECTS.try_lock().is_some());
                assert!(SCRATCH_WINDOWS.try_lock().is_some());
                assert_eq!(barriers.get(), 2, "scratch released before all barriers");
                assert_eq!(pages, 1);
                releases += 1;
                if asid == second {
                    assert_eq!(base, second_base);
                    // Reject before mutating the real allocator's reservation.
                    Err(MemoryObjectError::OutOfScratch)
                } else {
                    assert_eq!(asid, first);
                    assert_eq!(base, first_base);
                    release_scratch(asid, base, pages)
                }
            },
        );
        assert_eq!(releases, 2);
        assert_eq!(MEMORY_OBJECTS.lock().objects[&id].retirement_pins, 1);
        assert!(MEMORY_OBJECTS.lock().objects[&id].lend_state.references_cap(first, first_loan));
        assert!(MEMORY_OBJECTS.lock().objects[&id].lend_state.references_cap(second, second_loan));
        assert_eq!(close_cap(owner, cap), Err(MemoryObjectError::LendingActive));
        assert_eq!(budget::used(handle), amount(1));
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
        // Completed quiescent ranges can be reused; the rejected range cannot.
        let reused = reserve_scratch(first, 1).unwrap();
        assert_eq!(reused, first_base);
        let retained = reserve_scratch(second, 1).unwrap();
        assert_ne!(retained, second_base);
        release_scratch(first, reused, 1).unwrap();
        assert_eq!(release_scratch(first, reused, 1), Err(MemoryObjectError::OutOfScratch));
        release_scratch(second, retained, 1).unwrap();
    }
    close_test_address_space(first).unwrap();
    close_test_address_space(second).unwrap();
    close_test_address_space(owner).unwrap();
    assert_eq!(budget::used(handle), amount(1), "failed completion retains original charge");
}

fn test_batched_detach(create: &mut impl FnMut(&str) -> usize) {
    let owner = create("retirement batched owner");
    let borrower = create("retirement batched borrower");
    let pages = RETIREMENT_FRAME_BATCH * 2 + 3;
    let handle = current_address_space_handle(owner).unwrap();
    let cap = allocate(owner, pages).unwrap();
    let id = object_id(owner, cap);
    // Exercise standalone unmap and mapped-loan revoke using the same batches.
    map_any(owner, cap, false).unwrap();
    unmap(owner, cap).unwrap();
    let loan = lend_read(owner, cap, borrower).unwrap();
    map_any(borrower, loan, false).unwrap();
    revoke_lend(owner, cap, borrower, loan).unwrap();
    assert_eq!(info(borrower, loan), Err(MemoryObjectError::UnknownCapability));
    let base = map_any(owner, cap, false).unwrap();
    let mut copy = Some(pin_for_copy(owner, cap).unwrap());
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    {
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let receipt = RetiredObjectMappings::prepare(MEMORY_OBJECTS.lock(), id, owner);
        assert!(!receipt.detached);
        assert_eq!(
            receipt.pin.unmap_with(owner, base, pages + 1, |_, _, _| {
                panic!("invalid prefix reached the table walker")
            }),
            Err(MemoryObjectError::UnmapFailed)
        );
        let expected = MEMORY_OBJECTS.lock().objects[&id].frames[0];
        assert_eq!(
            ADDRESS_SPACE_TABLE.lock().get_mut(owner).unwrap().translate_address(base).unwrap(),
            expected
        );
        // Preparation fences authority/backing before any leaf is removed.
        assert_eq!(close_cap(owner, cap), Err(MemoryObjectError::LendingActive));
        let mut offset = 0;
        let mut batches = 0;
        let receipt = receipt.detach_with(|asid, address, frames| {
            assert_eq!(asid, owner);
            assert_eq!(address, base + offset * PAGE_SIZE);
            assert_eq!(frames.len(), (pages - offset).min(RETIREMENT_FRAME_BATCH));
            // Last unpin between metadata preparation and a table walk must
            // not release the backing or charge retained by this receipt.
            if let Some(pin) = copy.take() {
                unpin_copy(pin);
            }
            assert_eq!(budget::used(handle), amount(pages as u64));
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
            checked_unmap(asid, address, frames)?;
            offset += frames.len();
            batches += 1;
            Ok(())
        });
        assert!(receipt.detached);
        assert_eq!(offset, pages);
        assert_eq!(batches, 3);
        receipt.finish_with(owner, invalidate);
    }
    assert_eq!(budget::used(handle), budget::Amount::default());
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before + pages);
    close_test_address_space(owner).unwrap();
    close_test_address_space(borrower).unwrap();
}

fn test_last_unpin(create: &mut impl FnMut(&str) -> usize) {
    let owner = create("retirement pinned owner");
    let borrower = create("retirement pinned borrower");
    let handle = current_address_space_handle(owner).unwrap();
    let cap = allocate(owner, 1).unwrap();
    let id = object_id(owner, cap);
    map(owner, cap, VAddr::from(0x130000usize), false).unwrap();
    let loan = lend_read(owner, cap, borrower).unwrap();
    let base = map_any(borrower, loan, false).unwrap();
    let mut copy = Some(pin_for_copy(owner, cap).unwrap());
    let mut dma = Some(pin_for_dma(borrower, loan, true, false, false).unwrap());
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    {
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let receipt = detach(id, owner);
        assert!(receipt.detached);
        let mut barriers = 0;
        receipt.finish_with(owner, |asid, address, pages| {
            if let Some(pin) = copy.take() {
                unpin_copy(pin);
            }
            if let Some(pin) = dma.take() {
                unpin_dma(pin);
            }
            assert_eq!(budget::used(handle), amount(1));
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
            assert_eq!(write_bytes(owner, cap, &[1]), Err(MemoryObjectError::LendingActive));
            assert!(matches!(pin_for_copy(owner, cap), Err(MemoryObjectError::LendingActive)));
            assert!(matches!(
                pin_for_dma(borrower, loan, true, false, false),
                Err(MemoryObjectError::LendingActive)
            ));
            let temporary = reserve_scratch(borrower, 1).unwrap();
            assert_ne!(temporary, base, "scratch reused before all barriers");
            release_scratch(borrower, temporary, 1).unwrap();
            barriers += 1;
            invalidate(asid, address, pages)
        });
        assert_eq!(barriers, 2);
    }
    assert_eq!(budget::used(handle), budget::Amount::default());
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before + 1);
    let fresh = allocate(borrower, 1).unwrap();
    assert_eq!(map_any(borrower, fresh, true).unwrap(), base);
    unmap(borrower, fresh).unwrap();
    close_cap(borrower, fresh).unwrap();
    close_test_address_space(owner).unwrap();
    close_test_address_space(borrower).unwrap();
}

fn test_borrower_fence(create: &mut impl FnMut(&str) -> usize) {
    let owner = create("retirement live owner");
    let borrower = create("retirement write borrower");
    let cap = allocate(owner, 1).unwrap();
    let id = object_id(owner, cap);
    let loan = lend_write(owner, cap, borrower).unwrap();
    map_any(borrower, loan, true).unwrap();
    {
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let receipt = detach(id, borrower);
        receipt.finish_with(borrower, |asid, base, pages| {
            assert_eq!(write_bytes(owner, cap, &[2]), Err(MemoryObjectError::LendingActive));
            invalidate(asid, base, pages)
        });
    }
    write_bytes(owner, cap, &[3]).unwrap();
    close_test_address_space(borrower).unwrap();
    close_cap(owner, cap).unwrap();
    close_test_address_space(owner).unwrap();
}

fn test_failed_detach(create: &mut impl FnMut(&str) -> usize) {
    let owner = create("retirement partial detach");
    let handle = current_address_space_handle(owner).unwrap();
    let cap = allocate(owner, 2).unwrap();
    let id = object_id(owner, cap);
    let base = map_any(owner, cap, false).unwrap();
    let copy = pin_for_copy(owner, cap).unwrap();
    let dma = pin_for_dma(owner, cap, true, false, false).unwrap();
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    {
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let receipt = RetiredObjectMappings::prepare(MEMORY_OBJECTS.lock(), id, owner).detach_with(
            |asid, base, frames| {
                checked_unmap(asid, base, &frames[..1])?;
                Err(MemoryObjectError::UnmapFailed)
            },
        );
        assert!(!receipt.detached);
        unpin_copy(copy);
        unpin_dma(dma);
        receipt.finish_with(owner, invalidate);
        assert_eq!(budget::used(handle), amount(2));
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
        assert_eq!(close_cap(owner, cap), Err(MemoryObjectError::LendingActive));
        let frame = MEMORY_OBJECTS.lock().objects[&id].frames[1];
        let mut table = ADDRESS_SPACE_TABLE.lock();
        let space = table.get_mut(owner).unwrap();
        assert!(space.translate_address(base).is_err());
        assert_eq!(space.translate_address(base + PAGE_SIZE).unwrap(), frame);
    }
    close_test_address_space(owner).unwrap();
    assert_eq!(budget::used(handle), amount(2), "retired generation retains quarantine charge");
}

fn test_failed_barrier_and_abandonment(create: &mut impl FnMut(&str) -> usize) {
    for fail_barrier in [true, false] {
        let owner = create("retirement abandoned fence");
        let handle = current_address_space_handle(owner).unwrap();
        let cap = allocate(owner, 1).unwrap();
        let id = object_id(owner, cap);
        map_any(owner, cap, true).unwrap();
        let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        {
            let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
            let receipt = detach(id, owner);
            assert!(receipt.detached);
            if fail_barrier {
                receipt.finish_with(owner, |_, _, _| false);
            } else {
                drop(receipt);
            }
            assert_eq!(MEMORY_OBJECTS.lock().objects[&id].retirement_pins, 1);
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
            assert_eq!(budget::used(handle), amount(1));
        }
        close_test_address_space(owner).unwrap();
        assert_eq!(budget::used(handle), amount(1));
    }
}

fn test_failed_map_cleanup(create: &mut impl FnMut(&str) -> usize) {
    let owner = create("retirement failed map prefix");
    let handle = current_address_space_handle(owner).unwrap();
    let cap = allocate(owner, 2).unwrap();
    let foreign = allocate(owner, 1).unwrap();
    let id = object_id(owner, cap);
    let base = VAddr::from(0x140000usize);
    let foreign_id = object_id(owner, foreign);
    let foreign_frame = MEMORY_OBJECTS.lock().objects[&foreign_id].frames[0];
    map(owner, foreign, base + PAGE_SIZE, true).unwrap();
    {
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let mut pin = None;
        assert_eq!(
            map_locked(owner, cap, base, true, false, &mut pin),
            Err(MemoryObjectError::MapFailed)
        );
        assert!(!info(owner, cap).unwrap().mapped);
        assert_eq!(
            close_cap(owner, cap),
            Err(MemoryObjectError::LendingActive),
            "rolled-back prefix still owns backing before invalidation"
        );
        crate::cpu::isa::memory::tlb::inval_range_user(owner, base, 2);
        pin.take().unwrap().release(None);
        assert_eq!(MEMORY_OBJECTS.lock().objects[&id].retirement_pins, 0);
        let expected = MEMORY_OBJECTS.lock().objects[&id].frames[1];
        {
            let mut table = ADDRESS_SPACE_TABLE.lock();
            let space = table.get_mut(owner).unwrap();
            assert_eq!(
                unmap_owned_leaf(space, base + PAGE_SIZE, expected),
                Err(MemoryObjectError::UnmapFailed)
            );
            assert_eq!(space.translate_address(base + PAGE_SIZE).unwrap(), foreign_frame);
        }
        assert_eq!(
            map_locked_with_cleanup(owner, cap, base, true, false, &mut pin, |_, _, _| false),
            Err(MemoryObjectError::UnmapFailed)
        );
        crate::cpu::isa::memory::tlb::inval_range_user(owner, base, 2);
        drop(pin); // Failed cleanup does not release its backing pin.
        assert_eq!(MEMORY_OBJECTS.lock().objects[&id].mappings[&owner].installed_pages, 1);
        assert!(info(owner, cap).unwrap().mapped);
        assert_eq!(close_cap(owner, cap), Err(MemoryObjectError::LendingActive));
        let receipt = detach(id, owner);
        assert!(receipt.detached);
        // Retirement must not claim/unmap the collision page.
        assert_eq!(
            ADDRESS_SPACE_TABLE
                .lock()
                .get_mut(owner)
                .unwrap()
                .translate_address(base + PAGE_SIZE)
                .unwrap(),
            foreign_frame
        );
        receipt.finish_with(owner, invalidate);
    }
    unmap(owner, foreign).unwrap();
    close_cap(owner, foreign).unwrap();
    assert_eq!(budget::used(handle), amount(2));
    close_test_address_space(owner).unwrap();
    assert_eq!(budget::used(handle), amount(2));
}
