//! Boot fixtures for the owned loan transaction and both live root leases.
//! Fault/abandonment probes permanently retain four data pages and their charges.

use super::*;
use crate::{
    memory::{
        AddressSpaceCloseError,
        budget,
        close_user_address_space_handle,
        current_address_space_handle,
        retirement::{
            CloseProgress,
            ClosingAddressSpace,
        },
    },
    self_test::close_test_address_space,
};

fn used_page() -> budget::Amount {
    budget::Amount {
        pages: 1,
        objects: 1,
    }
}

fn checked_unmap(asid: usize, base: VAddr, frames: &[PAddr]) -> Result<(), MemoryObjectError> {
    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(), "loan detach held lifecycle");
    assert!(MEMORY_OBJECTS.try_lock().is_some(), "loan detach held registry");
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some(), "loan detach held table");
    unmap_pages(asid, base, frames)
}

fn checked_invalidate(asid: usize, base: VAddr, pages: usize) -> bool {
    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(), "loan barrier held lifecycle");
    assert!(MEMORY_OBJECTS.try_lock().is_some(), "loan barrier held registry");
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some(), "loan barrier held table");
    crate::cpu::isa::memory::tlb::inval_range_user(asid, base, pages);
    true
}

pub(crate) fn run(create: &mut impl FnMut(&str) -> usize) {
    test_success(create);
    test_preparation_errors(create);
    test_dma_pin_rejection(create);
    test_staged_close(create);
    test_failed_completion_and_drop(create);
    crate::logln!(
        "[loan revocation] both roots leased; lifecycle/registry/table-free detach and barrier, \
         reader preservation, preparation rollback, staged close and four failure/Drop \
         quarantines passed (four reserved data pages)"
    );
}

fn test_success(create: &mut impl FnMut(&str) -> usize) {
    let owner = create("revocation owner");
    let borrower = create("revocation borrower");
    let other = create("revocation remaining reader");
    let owner_handle = current_address_space_handle(owner).unwrap();
    let borrower_handle = current_address_space_handle(borrower).unwrap();
    let cap = allocate(owner, 1).unwrap();
    let loan = lend_read(owner, cap, borrower).unwrap();
    let remaining = lend_read(owner, cap, other).unwrap();
    let base = map_any(borrower, loan, false).unwrap();
    let mut copy = Some(pin_for_copy(owner, cap).unwrap());
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let operation = LeasedRevocation::prepare(owner, cap, borrower, loan).unwrap();
    assert_eq!(
        close_user_address_space_handle(owner_handle),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(
        close_user_address_space_handle(borrower_handle),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(close_cap(owner, cap), Err(MemoryObjectError::LendingActive));
    assert_eq!(close_cap(borrower, loan), Err(MemoryObjectError::LendingActive));
    assert_eq!(map_any(other, remaining, false), Err(MemoryObjectError::LendingActive));
    assert_eq!(revoke_lend(owner, cap, borrower, loan), Err(MemoryObjectError::LendingActive));
    let barrier_done = core::cell::Cell::new(false);
    operation
        .finish_with(|loan| {
            loan.finish_with(
                checked_unmap,
                |asid, address, pages| {
                    unpin_copy(copy.take().unwrap());
                    assert_eq!(budget::used(owner_handle), used_page());
                    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
                    let temporary = reserve_scratch(borrower, 1).unwrap();
                    assert_ne!(temporary, base, "loan scratch reused before barrier");
                    release_scratch(borrower, temporary, 1).unwrap();
                    barrier_done.set(true);
                    checked_invalidate(asid, address, pages)
                },
                |asid, address, pages| {
                    assert!(barrier_done.get());
                    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                    release_scratch(asid, address, pages)
                },
            )
        })
        .unwrap();
    assert_eq!(info(borrower, loan), Err(MemoryObjectError::UnknownCapability));
    assert!(info(other, remaining).unwrap().lent);
    let reused = reserve_scratch(borrower, 1).unwrap();
    assert_eq!(reused, base);
    release_scratch(borrower, reused, 1).unwrap();
    // The other read loan resumes after the transaction and survives this revoke.
    map_any(other, remaining, false).unwrap();
    unmap(other, remaining).unwrap();
    revoke_lend(owner, cap, other, remaining).unwrap();
    assert!(!info(owner, cap).unwrap().lent);
    close_cap(owner, cap).unwrap();
    assert_eq!(budget::used(owner_handle), budget::Amount::default());
    close_test_address_space(owner).unwrap();
    close_test_address_space(borrower).unwrap();
    close_test_address_space(other).unwrap();
}

fn test_preparation_errors(create: &mut impl FnMut(&str) -> usize) {
    let owner = create("revocation preparation owner");
    let borrower = create("revocation preparation borrower");
    let cap = allocate(owner, 1).unwrap();
    assert_eq!(revoke_lend(owner, cap, borrower, 0), Err(MemoryObjectError::NotLent));
    let loan = lend_write(owner, cap, borrower).unwrap();
    assert_eq!(
        revoke_lend(owner, cap, borrower, loan + 1),
        Err(MemoryObjectError::UnknownCapability)
    );
    // Invalid preparation did not fence the object or leave either root leased.
    write_bytes(borrower, loan, &[1]).unwrap();
    revoke_lend(owner, cap, borrower, loan).unwrap();
    close_cap(owner, cap).unwrap();
    close_test_address_space(owner).unwrap();
    close_test_address_space(borrower).unwrap();
}

fn test_dma_pin_rejection(create: &mut impl FnMut(&str) -> usize) {
    for writable in [false, true] {
        let owner = create("DMA-pinned loan owner");
        let borrower = create("DMA-pinned loan borrower");
        let cap = allocate(owner, 1).unwrap();
        let loan = if writable {
            lend_write(owner, cap, borrower).unwrap()
        } else {
            lend_read(owner, cap, borrower).unwrap()
        };
        let base = map_any(borrower, loan, writable).unwrap();
        let pin = pin_for_dma(borrower, loan, true, writable, false).unwrap();
        assert_eq!(revoke_lend(owner, cap, borrower, loan), Err(MemoryObjectError::LendingActive));
        assert!(info(owner, cap).unwrap().lent);
        assert!(info(borrower, loan).unwrap().mapped);
        assert_eq!(
            ADDRESS_SPACE_TABLE.lock().get_mut(borrower).unwrap().translate_address(base).unwrap(),
            pin.frames[0]
        );
        // Rejected preparation returns both leases and leaves the original
        // loan/pin intact. This is pin-state validation, not IOMMU fault injection.
        for asid in [owner, borrower] {
            AddressSpaceOperation::acquire(current_address_space_handle(asid).unwrap())
                .unwrap()
                .release()
                .unwrap();
        }
        unpin_dma(pin);
        revoke_lend(owner, cap, borrower, loan).unwrap();
        assert!(!info(owner, cap).unwrap().lent);
        close_cap(owner, cap).unwrap();
        close_test_address_space(owner).unwrap();
        close_test_address_space(borrower).unwrap();
    }
    crate::logln!("[loan revocation] read/write DMA pins reject revocation until explicit unpin");
}

fn test_staged_close(create: &mut impl FnMut(&str) -> usize) {
    let owner = create("revocation staged owner");
    let borrower = create("revocation staged borrower");
    let borrower_handle = current_address_space_handle(borrower).unwrap();
    let cap = allocate(owner, 1).unwrap();
    let loan = lend_write(owner, cap, borrower).unwrap();
    map_any(borrower, loan, true).unwrap();
    let operation = LeasedRevocation::prepare(owner, cap, borrower, loan).unwrap();
    let closing = ClosingAddressSpace::begin(borrower_handle).unwrap();
    let CloseProgress::Pending(closing) = closing.poll().unwrap() else {
        panic!("closing reclaimed borrower while revocation held its root");
    };
    assert!(matches!(
        AddressSpaceOperation::acquire(borrower_handle),
        Err(OperationError::Closing)
    ));
    operation
        .finish_with(|loan| loan.finish_with(checked_unmap, checked_invalidate, release_scratch))
        .unwrap();
    assert!(matches!(closing.poll().unwrap(), CloseProgress::Complete));
    close_cap(owner, cap).unwrap();
    close_test_address_space(owner).unwrap();

    // Acquiring the second root can fail after the first was already leased.
    let owner = create("revocation closing-admission owner");
    let borrower = create("revocation closing-admission borrower");
    let cap = allocate(owner, 1).unwrap();
    let loan = lend_read(owner, cap, borrower).unwrap();
    let closing =
        ClosingAddressSpace::begin(current_address_space_handle(borrower).unwrap()).unwrap();
    assert_eq!(
        revoke_lend(owner, cap, borrower, loan),
        Err(MemoryObjectError::AddressSpaceClosing)
    );
    // Closing the owner proves failed pair admission released its first lease.
    close_test_address_space(owner).unwrap();
    assert!(matches!(closing.poll().unwrap(), CloseProgress::Complete));
}

fn test_failed_completion_and_drop(create: &mut impl FnMut(&str) -> usize) {
    for stage in 0..4 {
        let owner = create("revocation failed owner");
        let borrower = create("revocation failed borrower");
        let handle = current_address_space_handle(owner).unwrap();
        let cap = allocate(owner, 1).unwrap();
        let loan = lend_write(owner, cap, borrower).unwrap();
        let id = MEMORY_OBJECTS.lock().lookup(owner, cap).unwrap().object;
        let base = map_any(borrower, loan, true).unwrap();
        let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let operation = LeasedRevocation::prepare(owner, cap, borrower, loan).unwrap();
        let result = operation.finish_with(|loan| {
            if stage == 3 {
                drop(loan);
                return Err(MemoryObjectError::UnmapFailed);
            }
            loan.finish_with(
                |asid, address, frames| {
                    if stage == 0 {
                        Err(MemoryObjectError::UnmapFailed)
                    } else {
                        checked_unmap(asid, address, frames)
                    }
                },
                |asid, address, pages| checked_invalidate(asid, address, pages) && stage != 1,
                |asid, address, pages| {
                    if stage == 2 {
                        Err(MemoryObjectError::OutOfScratch)
                    } else {
                        release_scratch(asid, address, pages)
                    }
                },
            )
        });
        assert!(result.is_err());
        assert_eq!(MEMORY_OBJECTS.lock().objects[&id].retirement_pins, 1);
        assert!(matches!(MEMORY_OBJECTS.lock().objects[&id].lend_state, LendState::Revoking));
        assert!(info(borrower, loan).unwrap().mapped);
        assert_eq!(close_cap(owner, cap), Err(MemoryObjectError::LendingActive));
        assert_eq!(close_cap(borrower, loan), Err(MemoryObjectError::LendingActive));
        assert_eq!(write_bytes(owner, cap, &[2]), Err(MemoryObjectError::LendingActive));
        assert_eq!(budget::used(handle), used_page());
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
        let fresh = reserve_scratch(borrower, 1).unwrap();
        assert_ne!(fresh, base, "failed loan completion reused scratch");
        release_scratch(borrower, fresh, 1).unwrap();
        close_test_address_space(borrower).unwrap();
        close_test_address_space(owner).unwrap();
        assert_eq!(budget::used(handle), used_page(), "failed loan completion refunded charge");
        assert!(MEMORY_OBJECTS.lock().objects.contains_key(&id));
    }
}
