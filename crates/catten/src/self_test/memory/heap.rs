use crate::{
    cpu::isa::interface::memory::AddressSpaceInterface,
    memory::{
        self,
        ADDRESS_SPACE_TABLE,
        PHYSICAL_FRAME_ALLOCATOR,
        linear::VAddr,
    },
    service::loader,
};

pub fn test_heap_admission() {
    memory::backing_budget::test_pool();
    let handle = loader::create_user_address_space_handle();
    let page = charlotte_launch::HEAP_VADDR;
    let used = memory::backing_budget::test_used_pages(memory::backing_budget::Kind::Heap);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert!(!memory::commit_user_heap_page_with_mapper(handle, page, |_, _| false));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(memory::backing_budget::test_used_pages(memory::backing_budget::Kind::Heap), used);
    ADDRESS_SPACE_TABLE.lock().get_mut(handle.id()).unwrap().heap_account.set_limit(1).unwrap();
    assert!(!memory::commit_user_heap_page_handle(handle, page - 1));
    assert!(memory::commit_user_heap_page_handle(handle, page));
    let frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(handle.id())
        .unwrap()
        .translate_address(VAddr::from(page))
        .unwrap();
    let hhdm: *const u8 = frame.into();
    assert!(unsafe { core::slice::from_raw_parts(hhdm, 4096) }.iter().all(|&byte| byte == 0));
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert!(memory::commit_user_heap_page_handle(handle, page + 1));
    assert!(!memory::commit_user_heap_page_handle(handle, page + 4096));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(ADDRESS_SPACE_TABLE.lock().get_mut(handle.id()).unwrap().heap_account.pages(), 1);
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(memory::backing_budget::test_used_pages(memory::backing_budget::Kind::Heap), used);
    let replacement = loader::create_user_address_space_handle();
    assert_eq!(replacement.id(), handle.id());
    assert_ne!(replacement, handle);
    assert!(!memory::commit_user_heap_page_handle(handle, page));
    assert_eq!(
        ADDRESS_SPACE_TABLE.lock().get_mut(replacement.id()).unwrap().heap_account.pages(),
        0
    );
    assert!(memory::commit_user_heap_page_handle(replacement, page));
    // Logical retirement refuses new backing but doesn't release live frames.
    ADDRESS_SPACE_TABLE.lock().get_mut(replacement.id()).unwrap().heap_account.retire();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert!(!memory::commit_user_heap_page_handle(replacement, page));
    assert!(!memory::commit_user_heap_page_handle(replacement, page + 4096));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    memory::close_user_address_space_handle(replacement).unwrap();
    assert_eq!(memory::backing_budget::test_used_pages(memory::backing_budget::Kind::Heap), used);
    crate::logln!(
        "[heap admission] pool reserve/refund, zero backing, quota, repeated touch, retirement \
         and exact ASID reuse passed"
    );
}
