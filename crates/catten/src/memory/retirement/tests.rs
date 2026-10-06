//! Single-mutator boot probes. Fault adapters must not recover quarantined
//! roots or return their slots through a test-only bypass.

use super::*;
use crate::{
    memory::{
        self,
        ADDRESS_SPACE_LIFECYCLE,
        PHYSICAL_FRAME_ALLOCATOR,
        backing_budget::{
            self,
            Kind,
        },
        operation::{
            AddressSpaceOperation,
            OperationError,
        },
    },
    service::loader,
};

fn stage(handle: AddressSpaceHandle) -> RetiredAddressSpace {
    match ClosingAddressSpace::begin_ready(handle).unwrap().prepare_retirement().unwrap() {
        RetirementProgress::Ready(retired) => retired,
        RetirementProgress::Pending(_) => panic!("ready fixture retirement unexpectedly pending"),
    }
}

fn assert_detached(handle: AddressSpaceHandle) {
    assert!(
        ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(),
        "lifecycle guard leaked into final invalidation"
    );
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some(), "table guard leaked into final invalidation");
    assert_eq!(memory::current_address_space_handle(handle.id()), None);
    assert!(!memory::commit_user_heap_page_handle(handle, charlotte_launch::HEAP_VADDR));
    assert_eq!(
        memory::close_user_address_space_handle(handle),
        Err(AddressSpaceCloseError::AddressSpaceMissing)
    );
}

pub(crate) fn run() {
    test_live_operations();
    test_abandoned_operation();
    test_staged_close();
    test_staged_close_timeout();
    test_preflight_failure();
    test_successful_retirement();
    test_quarantine(false);
    test_quarantine(true);
    crate::logln!(
        "[root retirement] preflight rollback, post-guard invalidation, retained backing/charges, \
         leased slot and hardware tag, exact reuse, failed barrier and Drop quarantine passed \
         (two retired roots, one retained live root and one retained closing root)"
    );
}

fn test_staged_close() {
    let before = backing_budget::test_used_pages(Kind::Heap);
    let owner = loader::create_user_address_space_handle();
    assert!(memory::commit_user_heap_page_handle(owner, charlotte_launch::HEAP_VADDR));
    let object = memory::object::allocate(owner.id(), 1).unwrap();
    memory::object::map_any(owner.id(), object, true).unwrap();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let first = AddressSpaceOperation::acquire(owner).unwrap();
    let second = AddressSpaceOperation::acquire(owner).unwrap();
    let closing = ClosingAddressSpace::begin(owner).unwrap();
    assert!(matches!(AddressSpaceOperation::acquire(owner), Err(OperationError::Closing)));
    assert!(matches!(
        ClosingAddressSpace::begin(owner),
        Err(AddressSpaceCloseError::CloseInProgress)
    ));
    assert_eq!(
        memory::close_user_address_space_handle(owner),
        Err(AddressSpaceCloseError::CloseInProgress)
    );
    let CloseProgress::Pending(closing) =
        closing.poll_with(|_, _| panic!("pending close must not invalidate translations")).unwrap()
    else {
        panic!("close completed with two outstanding operations");
    };
    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
    assert_eq!(memory::current_address_space_handle(owner.id()), Some(owner));
    assert!(memory::budget::accepting(owner));
    assert!(memory::object::info(owner.id(), object).unwrap().mapped);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
    first.release().unwrap();
    let CloseProgress::Pending(closing) = closing.poll().unwrap() else {
        panic!("close completed with one outstanding operation");
    };
    // Older operations can complete after admission is fenced, without taking
    // lifecycle or accidentally making the slot available to another close.
    {
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        second.release().unwrap();
    }
    assert!(matches!(AddressSpaceOperation::acquire(owner), Err(OperationError::Closing)));
    assert!(matches!(
        closing
            .poll_with(|space, handle| {
                assert_detached(handle);
                assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
                invalidate(space, handle)
            })
            .unwrap(),
        CloseProgress::Complete
    ));
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before);
    let replacement = loader::create_user_address_space_handle();
    assert_eq!(replacement.id(), owner.id());
    assert_ne!(replacement, owner);
    AddressSpaceOperation::acquire(replacement).unwrap().release().unwrap();
    // A ready request may complete even with a zero waiting budget.
    ClosingAddressSpace::begin(replacement).unwrap().wait(0).unwrap();
}

fn test_staged_close_timeout() {
    let before = backing_budget::test_used_pages(Kind::Heap);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let owner = loader::create_user_address_space_handle();
    assert!(memory::commit_user_heap_page_handle(owner, charlotte_launch::HEAP_VADDR));
    let retained = free - PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let operation = AddressSpaceOperation::acquire(owner).unwrap();
    #[cfg(target_arch = "aarch64")]
    let tag = ADDRESS_SPACE_TABLE.lock().get(owner.id()).unwrap().hw_asid();
    assert_eq!(
        ClosingAddressSpace::begin(owner).unwrap().wait(0),
        Err(AddressSpaceCloseError::OperationDrainTimedOut)
    );
    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
    operation.release().unwrap();
    // Timeout abandons only the request, not its fence or root. Draining the
    // last lease afterwards cannot resurrect admission or steal close authority.
    assert!(matches!(AddressSpaceOperation::acquire(owner), Err(OperationError::Closing)));
    assert_eq!(
        memory::close_user_address_space_handle(owner),
        Err(AddressSpaceCloseError::CloseInProgress)
    );
    assert!(matches!(
        ClosingAddressSpace::begin(owner),
        Err(AddressSpaceCloseError::CloseInProgress)
    ));
    assert_eq!(memory::current_address_space_handle(owner.id()), Some(owner));
    assert!(memory::budget::accepting(owner));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - retained);
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
    #[cfg(target_arch = "aarch64")]
    assert_eq!(ADDRESS_SPACE_TABLE.lock().get(owner.id()).unwrap().hw_asid(), tag);
    let other = loader::create_user_address_space_handle();
    assert_ne!(other.id(), owner.id());
    memory::close_user_address_space_handle(other).unwrap();
    crate::logln!(
        "[staged close] lease admission fence, pending owner return, old-operation completion, \
         post-guard release and bounded timeout passed; retained_frames={} heap_pages=1",
        retained
    );
}

fn test_live_operations() {
    let before = backing_budget::test_used_pages(Kind::Heap);
    let owner = loader::create_user_address_space_handle();
    assert!(memory::commit_user_heap_page_handle(owner, charlotte_launch::HEAP_VADDR));
    let object = memory::object::allocate(owner.id(), 1).unwrap();
    memory::object::map_any(owner.id(), object, true).unwrap();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let first = AddressSpaceOperation::acquire(owner).unwrap();
    let second = AddressSpaceOperation::acquire(owner).unwrap();
    assert_eq!(first.handle(), owner);
    assert_eq!(second.handle(), owner);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    #[cfg(target_arch = "aarch64")]
    let tag = ADDRESS_SPACE_TABLE.lock().get(owner.id()).unwrap().hw_asid();
    assert_eq!(
        memory::close_user_address_space_handle(owner),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(memory::current_address_space_handle(owner.id()), Some(owner));
    assert!(memory::budget::accepting(owner));
    assert!(memory::object::info(owner.id(), object).unwrap().mapped);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
    #[cfg(target_arch = "aarch64")]
    assert_eq!(ADDRESS_SPACE_TABLE.lock().get(owner.id()).unwrap().hw_asid(), tag);
    // Busy close has not retired heap/capability/backing admission.
    assert!(memory::commit_user_heap_page_handle(owner, charlotte_launch::HEAP_VADDR + 4096));
    let fresh = memory::object::allocate(owner.id(), 1).unwrap();
    memory::object::close_cap(owner.id(), fresh).unwrap();
    let intervening = loader::create_user_address_space_handle();
    assert_ne!(intervening.id(), owner.id());
    first.release().unwrap();
    assert_eq!(
        memory::close_user_address_space_handle(owner),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    // Completion itself does not acquire lifecycle or initiate invalidation.
    {
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        second.release().unwrap();
    }
    let retired = stage(owner);
    assert!(matches!(
        AddressSpaceOperation::acquire(owner),
        Err(OperationError::AddressSpaceMissing)
    ));
    retired.release().unwrap();
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before);
    let replacement = loader::create_user_address_space_handle();
    assert_eq!(replacement.id(), owner.id());
    assert!(matches!(AddressSpaceOperation::acquire(owner), Err(OperationError::StaleHandle)));
    assert!(matches!(
        AddressSpaceOperation::acquire(
            memory::current_address_space_handle(memory::KERNEL_ASID).unwrap()
        ),
        Err(OperationError::KernelAddressSpace)
    ));
    memory::close_user_address_space_handle(replacement).unwrap();
    memory::close_user_address_space_handle(intervening).unwrap();
}

fn test_abandoned_operation() {
    let before = backing_budget::test_used_pages(Kind::Heap);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let owner = loader::create_user_address_space_handle();
    assert!(memory::commit_user_heap_page_handle(owner, charlotte_launch::HEAP_VADDR));
    let retained = free - PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    drop(AddressSpaceOperation::acquire(owner).unwrap());
    assert_eq!(
        memory::close_user_address_space_handle(owner),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(memory::current_address_space_handle(owner.id()), Some(owner));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - retained);
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
    let other = loader::create_user_address_space_handle();
    assert_ne!(other.id(), owner.id());
    memory::close_user_address_space_handle(other).unwrap();
    crate::logln!(
        "[live operations] explicit completion, busy-close rollback, stale/detached rejection and \
         abandonment passed; retained_frames={} heap_pages=1",
        retained
    );
}

fn test_preflight_failure() {
    let before = backing_budget::test_used_pages(Kind::Heap);
    let owner = loader::create_user_address_space_handle();
    assert!(memory::commit_user_heap_page_handle(owner, charlotte_launch::HEAP_VADDR));
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    {
        assert!(matches!(
            ClosingAddressSpace::begin_with(owner, |_, _| {
                Err(crate::klib::collections::id_table::Error::AllocationFailed)
            }),
            Err(AddressSpaceCloseError::RetirementMetadataAllocationFailed)
        ));
    }
    assert_eq!(memory::current_address_space_handle(owner.id()), Some(owner));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
    assert!(memory::commit_user_heap_page_handle(owner, charlotte_launch::HEAP_VADDR + 4096));
    // Subsystem admission remains active too: no partial namespace retirement.
    let object = memory::object::allocate(owner.id(), 1).unwrap();
    memory::object::close_cap(owner.id(), object).unwrap();
    memory::close_user_address_space_handle(owner).unwrap();
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before);
}

fn test_successful_retirement() {
    let before = backing_budget::test_used_pages(Kind::Heap);
    let start_free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let owner = loader::create_user_address_space_handle();
    assert!(memory::commit_user_heap_page_handle(owner, charlotte_launch::HEAP_VADDR));
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let physical_pages = start_free - free;
    let retired = stage(owner);
    assert_detached(owner);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
    let intervening = loader::create_user_address_space_handle();
    assert_ne!(intervening.id(), owner.id(), "detached slot recycled before quiescence");
    #[cfg(target_arch = "aarch64")]
    assert_ne!(
        ADDRESS_SPACE_TABLE.lock().get(intervening.id()).unwrap().hw_asid(),
        retired.entry.value().hw_asid()
    );
    // Avoid attributing the intervening root allocation to retired backing.
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    retired
        .release_with(|space, handle| {
            assert_detached(handle);
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
            assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
            invalidate(space, handle)
        })
        .unwrap();
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free + physical_pages);
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before);
    let replacement = loader::create_user_address_space_handle();
    assert_eq!(replacement.id(), owner.id());
    assert_ne!(replacement, owner);
    assert_eq!(
        memory::close_user_address_space_handle(owner),
        Err(AddressSpaceCloseError::StaleHandle)
    );
    assert_eq!(memory::current_address_space_handle(replacement.id()), Some(replacement));
    memory::close_user_address_space_handle(replacement).unwrap();
    memory::close_user_address_space_handle(intervening).unwrap();
}

fn test_quarantine(abandon: bool) {
    let before = backing_budget::test_used_pages(Kind::Heap);
    let start_free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let owner = loader::create_user_address_space_handle();
    assert!(memory::commit_user_heap_page_handle(owner, charlotte_launch::HEAP_VADDR));
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let physical_pages = start_free - free;
    let retired = stage(owner);
    assert_detached(owner);
    if abandon {
        // RetiredEntry's ManuallyDrop owns the resource even though the
        // receipt's ordinary metadata is dropped. No address-space destructor.
        drop(retired);
    } else {
        assert_eq!(
            retired.release_with(|_, handle| {
                assert_detached(handle);
                false
            }),
            Err(AddressSpaceCloseError::QuiescenceFailed)
        );
    }
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
    let replacement = loader::create_user_address_space_handle();
    assert_ne!(replacement.id(), owner.id());
    memory::close_user_address_space_handle(replacement).unwrap();
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
    crate::logln!(
        "[root retirement] quarantine abandon={} retained_frames={} heap_pages=1",
        abandon,
        physical_pages
    );
}
