//! Single-mutator kernel fixtures use the real walkers and destructor owners.
use super::*;
use crate::memory::{
    AddressSpace,
    AddressSpaceInterface,
    PHYSICAL_FRAME_ALLOCATOR,
    VAddr,
    linear::{
        MemoryMapping,
        PageType,
    },
};

fn map(space: &mut AddressSpace, vaddr: usize, frame: PAddr) -> bool {
    space
        .map_existing_page(MemoryMapping {
            vaddr: VAddr::from(vaddr),
            paddr: frame,
            page_type: PageType::UserData,
        })
        .is_ok()
}

pub(super) fn run() {
    sparse_domains();
    object_mapping();
    platform_progress();
    provisional_retention();
    crate::logln!(
        "[table admission] sparse domain ceiling, public object map rejection/cleanup, \
         partial-tree charge/retry, cached reuse, independent roots, ordinary-pressure platform \
         root/mapping and exact teardown passed; rejected/abandoned preparation retains two \
         frames/charges"
    );
}

fn object_mapping() {
    use crate::memory::{
        ADDRESS_SPACE_TABLE,
        budget,
        object,
    };
    // Raw kernel ABI fixture: the captured root owns the capability and backing
    // cleanup. No userspace owner or successor adopts this scalar capability.
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let used = account::test_used_pages();
    let owner = crate::service::loader::create_user_address_space_handle();
    ADDRESS_SPACE_TABLE.lock().get_mut(owner.id()).unwrap().table_account.set_limit(5).unwrap();
    let memory = object::allocate(owner.id(), 1).unwrap();
    let backing = budget::used(owner);
    object::map(owner.id(), memory, VAddr::from(0x4000_0000usize), true).unwrap();
    object::unmap(owner.id(), memory).unwrap();
    for _ in 0..16 {
        assert_eq!(
            object::map(owner.id(), memory, VAddr::from(0x8000_0000usize), true),
            Err(object::MemoryObjectError::MapFailed)
        );
        assert_eq!(budget::used(owner), backing);
        assert!(!object::info(owner.id(), memory).unwrap().mapped);
        object::write_bytes(owner.id(), memory, &[0x5a]).unwrap();
        assert_eq!(ADDRESS_SPACE_TABLE.lock().get(owner.id()).unwrap().table_account.pages(), 5);
        object::map(owner.id(), memory, VAddr::from(0x4000_0000usize), true).unwrap();
        object::unmap(owner.id(), memory).unwrap();
    }
    // Mapping rollback released its pins and root leases; ordinary close owns
    // the object and exact retained table hierarchy without any recovery bypass.
    object::close_cap(owner.id(), memory).unwrap();
    crate::memory::close_user_address_space_handle(owner).unwrap();
    assert_eq!(account::test_used_pages(), used);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
}

fn sparse_domains() {
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let used = account::test_used_pages();
    let ordinary = account::test_ordinary_pages();
    let data = PreparingUserFrame::allocate_zeroed().unwrap();
    let mut first = AddressSpace::try_new_user().unwrap();
    let mut second = AddressSpace::try_new_user().unwrap();
    first.table_account.set_limit(5).unwrap();
    assert!(map(&mut first, 0x4000_0000, data.frame()));
    assert_eq!(first.table_account.pages(), 4);
    first.unmap_page(VAddr::from(0x4000_0000usize)).unwrap();
    assert_eq!(first.table_account.pages(), 4);
    // A new 1-GiB region needs two tables. Its admitted prefix remains owned.
    assert!(!map(&mut first, 0x8000_0000, data.frame()));
    assert_eq!(first.table_account.pages(), 5);
    assert!(!first.is_mapped(VAddr::from(0x8000_0000usize)).unwrap());
    let at_limit = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert!(!map(&mut first, 0x8000_0000, data.frame()));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), at_limit);
    // Cached branches need no fresh admission, even at the exact ceiling.
    for _ in 0..16 {
        assert!(map(&mut first, 0x4000_0000, data.frame()));
        first.unmap_page(VAddr::from(0x4000_0000usize)).unwrap();
    }
    assert_eq!(first.table_account.pages(), 5);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), at_limit);
    assert!(map(&mut second, 0x8000_0000, data.frame()));
    assert_eq!(second.table_account.pages(), 4);
    first.table_account.set_limit(6).unwrap();
    assert!(map(&mut first, 0x8000_0000, data.frame()));
    assert_eq!(first.table_account.pages(), 6);
    assert_eq!(account::test_used_pages(), used + 10);
    drop(first);
    assert_eq!(account::test_used_pages(), used + 4);
    assert_eq!(second.translate_address(VAddr::from(0x8000_0000usize)).unwrap(), data.frame());
    drop(second);
    drop(data);
    assert_eq!(account::test_used_pages(), used);
    assert_eq!(account::test_ordinary_pages(), ordinary);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
}

fn platform_progress() {
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let used = account::test_used_pages();
    let ordinary = account::test_ordinary_pages();
    let data = PreparingUserFrame::allocate_zeroed().unwrap();
    let pressure = account::OrdinaryPressure::new();
    let ordinary_root = AddressSpace::try_new_user();
    #[cfg(target_arch = "x86_64")]
    assert!(ordinary_root.is_err());
    #[cfg(target_arch = "aarch64")]
    {
        let mut root = ordinary_root.unwrap();
        assert!(!map(&mut root, 0x4000_0000, data.frame()));
        assert_eq!(root.hw_asid(), 0);
        assert_eq!(root.table_account.pages(), 0);
    }
    // Classification must precede x86 root admission, not just namespace grant.
    let mut platform = AddressSpace::try_new_platform_user().unwrap();
    assert!(map(&mut platform, 0x4000_0000, data.frame()));
    assert_eq!(platform.table_account.pages(), 4);
    assert_eq!(account::test_used_pages(), used + 4);
    assert_eq!(account::test_ordinary_pages(), ordinary);
    drop(platform);
    drop(pressure);
    let mut retry = AddressSpace::try_new_user().unwrap();
    assert!(map(&mut retry, 0x4000_0000, data.frame()));
    drop(retry);
    drop(data);
    assert_eq!(account::test_used_pages(), used);
    assert_eq!(account::test_ordinary_pages(), ordinary);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
}

fn provisional_retention() {
    for abandoned in [false, true] {
        let used = account::test_used_pages();
        let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let mut space = AddressSpace::try_new_user().unwrap();
        let initial = space.table_account.pages();
        let account = &mut space.table_account;
        assert!(
            PreparingTable::allocate_with(TableScope::PrivateUser, Some(account), |_| None)
                .is_none()
        );
        assert_eq!(account.pages(), initial);
        account.set_limit(initial + 1).unwrap();
        let mut preparation =
            PreparingTable::allocate(TableScope::PrivateUser, Some(account)).unwrap();
        if abandoned {
            // Emulate interruption after disarm but before publication returns.
            preparation.state = PreparationState::Publishing;
            preparation.frame.take().unwrap().quarantine();
        } else {
            preparation.rollback_with(|_| {
                Err(crate::memory::physical::Error::CannotDeallocateUnallocatedFrame)
            });
        }
        drop(preparation);
        assert_eq!(account.pages(), initial + 1);
        assert!(PreparingTable::allocate(TableScope::PrivateUser, Some(account)).is_none());
        drop(space);
        assert_eq!(account::test_used_pages(), used + 1);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 1);
    }
}
