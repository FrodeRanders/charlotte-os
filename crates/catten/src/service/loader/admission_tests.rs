use super::*;
use crate::memory::{
    self,
    PHYSICAL_FRAME_ALLOCATOR,
    backing_budget::{
        self,
        Kind,
    },
};

pub fn test_admission() {
    test_validation();
    let before = backing_budget::test_used_pages(Kind::Image);
    let heap_before = backing_budget::test_used_pages(Kind::Heap);
    let handle = create_user_address_space_handle();
    let page = 0x0002_0000;
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert_eq!(
        map_image_page_with_mapper(handle, page, PageType::UserRoData, |_| {}, |_, _| false),
        Err(DomainLoadError::PageMapping)
    );
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(backing_budget::test_used_pages(Kind::Image), before);
    ADDRESS_SPACE_TABLE.lock().get_mut(handle.id()).unwrap().image_account.set_limit(1).unwrap();
    let frame = map_image_page(handle, page, PageType::UserRoData, |bytes| {
        bytes[..4].copy_from_slice(b"ELF!")
    })
    .unwrap();
    let hhdm: *const u8 = frame.into();
    let bytes = unsafe { core::slice::from_raw_parts(hhdm, PAGE_SIZE) };
    assert_eq!(&bytes[..4], b"ELF!");
    assert!(bytes[4..].iter().all(|&byte| byte == 0));
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert_eq!(
        map_image_page(handle, page, PageType::UserData, |_| {}),
        Err(DomainLoadError::PageMapping)
    );
    assert_eq!(
        map_image_page(handle, page + PAGE_SIZE, PageType::UserData, |_| {}),
        Err(DomainLoadError::BackingAdmission)
    );
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), heap_before);
    assert_eq!(backing_budget::test_used_pages(Kind::Image), before + 1);
    ADDRESS_SPACE_TABLE.lock().get_mut(handle.id()).unwrap().image_account.retire();
    assert_eq!(
        map_image_page(handle, page + PAGE_SIZE, PageType::UserData, |_| {}),
        Err(DomainLoadError::StaleAddressSpace)
    );
    assert_eq!(backing_budget::test_used_pages(Kind::Image), before + 1);
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(backing_budget::test_used_pages(Kind::Image), before);
    let replacement = create_user_address_space_handle();
    assert_eq!(replacement.id(), handle.id());
    assert_ne!(replacement, handle);
    assert_eq!(
        map_image_page(handle, page, PageType::UserData, |_| {}),
        Err(DomainLoadError::StaleAddressSpace)
    );
    map_image_page(replacement, page, PageType::UserData, |_| {}).unwrap();
    memory::close_user_address_space_handle(replacement).unwrap();
    assert_eq!(backing_budget::test_used_pages(Kind::Image), before);

    // Partial signed-image preparation fails after its first mapped page. The
    // unstarted owner must return backing and the exact namespace, not panic.
    let image = crate::service::store::service_elf(b"ns").unwrap();
    let expected = image_backing_pages(image);
    assert!(expected > 8);
    let cq_before = crate::completion::cq_budget::node_used();
    let prepared = core::cell::Cell::new(None);
    assert!(matches!(
        try_load_domain_with_preparation(
            image,
            &charlotte_launch::CLUSTER_PUBLIC_KEY,
            false,
            |handle| {
                prepared.set(Some(handle));
                ADDRESS_SPACE_TABLE
                    .lock()
                    .get_mut(handle.id())
                    .unwrap()
                    .image_account
                    .set_limit(1)
                    .unwrap();
                Ok(())
            }
        ),
        Err(DomainLoadError::BackingAdmission)
    ));
    assert!(!memory::address_space_handle_is_current(prepared.get().unwrap()));
    assert_eq!(backing_budget::test_used_pages(Kind::Image), before);
    assert_eq!(crate::completion::cq_budget::node_used(), cq_before);
    let loaded = try_load_domain(image).unwrap();
    assert_eq!(loaded.address_space.id(), prepared.get().unwrap().id());
    assert_eq!(
        ADDRESS_SPACE_TABLE.lock().get_mut(loaded.asid).unwrap().image_account.pages(),
        expected
    );
    assert_eq!(backing_budget::test_used_pages(Kind::Image), before + expected);
    memory::close_user_address_space_handle(loaded.address_space).unwrap();
    assert_eq!(backing_budget::test_used_pages(Kind::Image), before);
    assert_eq!(crate::completion::cq_budget::node_used(), cq_before);
    crate::logln!(
        "[loader admission] bounded validation, fill/zero, quota, mapping rollback, \
         retirement/reuse and signed partial/successful launch cleanup passed"
    );
}

fn test_validation() {
    // One executable BSS page, enough to exercise layout without a signature.
    let mut image = [0u8; 120];
    image[..4].copy_from_slice(ELF_MAGIC);
    image[4] = ELFCLASS64;
    image[5] = ELFDATA2LSB;
    image[16..18].copy_from_slice(&ET_EXEC.to_le_bytes());
    image[18..20].copy_from_slice(&expected_machine().to_le_bytes());
    image[24..32].copy_from_slice(&0x0002_0000u64.to_le_bytes());
    image[32..40].copy_from_slice(&64u64.to_le_bytes());
    image[54..56].copy_from_slice(&56u16.to_le_bytes());
    image[56..58].copy_from_slice(&1u16.to_le_bytes());
    image[64..68].copy_from_slice(&PT_LOAD.to_le_bytes());
    image[68..72].copy_from_slice(&PF_X.to_le_bytes());
    image[80..88].copy_from_slice(&0x0002_0000u64.to_le_bytes());
    image[104..112].copy_from_slice(&(PAGE_SIZE as u64).to_le_bytes());
    assert!(validate_user_elf(&image));
    assert_eq!(image_backing_pages(&image), 9);
    let mut many_headers = [0u8; 64 + 56 * (MAX_ELF_PROGRAM_HEADERS + 1)];
    many_headers[..image.len()].copy_from_slice(&image);
    many_headers[56..58].copy_from_slice(&((MAX_ELF_PROGRAM_HEADERS + 1) as u16).to_le_bytes());
    assert!(!validate_user_elf(&many_headers));
    let gap = (HEAP_VADDR + charlotte_launch::HEAP_SIZE) as u64;
    image[24..32].copy_from_slice(&gap.to_le_bytes());
    image[80..88].copy_from_slice(&gap.to_le_bytes());
    assert!(!validate_user_elf(&image), "ELF cannot occupy the adaptive heap extension");
    image[24..32].copy_from_slice(&0x0100_0000u64.to_le_bytes());
    image[80..88].copy_from_slice(&0x0100_0000u64.to_le_bytes());
    image[104..112]
        .copy_from_slice(&(backing_budget::IMAGE_DOMAIN_PAGES * PAGE_SIZE as u64).to_le_bytes());
    assert!(validate_user_elf(&image));
    assert!(image_backing_pages(&image) > backing_budget::IMAGE_DOMAIN_PAGES);
}
