//! Serialized boot fault adapters, not real OOM or panic/unwind tests.

use super::*;
use crate::{
    memory::{
        self,
        ADDRESS_SPACE_TABLE,
        AddressSpaceInterface,
        backing_budget,
    },
    service::loader,
};

pub(crate) fn run() {
    for kind in [Kind::Heap, Kind::Image] {
        test_preparation(kind);
        for platform in [false, true] {
            test_rejected_release(kind, platform);
        }
        for phase in [0, 1, 2] {
            test_unconfirmed_publication(kind, phase);
        }
        test_abandoned_release(kind);
    }
    crate::logln!(
        "[backing preparation test] admission/tracking/allocation rejection, Drop and mapping \
         rollback, fill/success, failed release/domain ceiling, pool identity, mixed \
         teardown/reuse, unconfirmed publication and abandoned receipt passed (12 retained \
         frames; 6 heap and 6 image charges)"
    );
}

fn test_preparation(kind: Kind) {
    let used = backing_budget::test_used_pages(kind);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let mut space = AddressSpace::try_new_user().unwrap();
    let with_root = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    account(&mut space, kind).set_limit(0).unwrap();
    assert!(matches!(
        PreparingUserBacking::new_with(
            &mut space,
            kind,
            |_| panic!("tracking after admission rejection"),
            || panic!("allocation after admission rejection")
        ),
        Err(BackingPreparationError::Admission)
    ));
    account(&mut space, kind).set_limit(1).unwrap();
    assert!(matches!(
        PreparingUserBacking::new_with(
            &mut space,
            kind,
            |_| false,
            || panic!("allocation after tracking rejection")
        ),
        Err(BackingPreparationError::Tracking)
    ));
    assert!(matches!(
        PreparingUserBacking::new_with(
            &mut space,
            kind,
            |space| space.prepare_user_frame().is_ok(),
            || None
        ),
        Err(BackingPreparationError::Allocation)
    ));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), with_root);
    assert_eq!(backing_budget::test_used_pages(kind), used);
    assert_eq!(account(&mut space, kind).pages(), 0);

    let preparation = PreparingUserBacking::new(&mut space, kind).unwrap();
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), with_root - 1);
    assert_eq!(backing_budget::test_used_pages(kind), used + 1);
    drop(preparation);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), with_root);
    assert_eq!(backing_budget::test_used_pages(kind), used);

    assert!(
        PreparingUserBacking::new(&mut space, kind)
            .unwrap()
            .map_with(VAddr::from(charlotte_launch::HEAP_VADDR), PageType::UserData, |_, _| false,)
            .is_err()
    );
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), with_root);
    assert_eq!(backing_budget::test_used_pages(kind), used);
    assert_eq!(account(&mut space, kind).pages(), 0);

    let mut preparation = PreparingUserBacking::new(&mut space, kind).unwrap();
    preparation.fill(|bytes| {
        assert!(bytes.iter().all(|&byte| byte == 0));
        bytes[..4].copy_from_slice(b"PREP");
    });
    let frame = preparation
        .map_with(
            VAddr::from(charlotte_launch::HEAP_VADDR),
            PageType::UserData,
            |space, mapping| space.map_existing_page(mapping).is_ok(),
        )
        .unwrap();
    let bytes: *const u8 = frame.into();
    assert_eq!(unsafe { core::slice::from_raw_parts(bytes, 4) }, b"PREP");
    assert_eq!(account(&mut space, kind).pages(), 1);
    assert_eq!(backing_budget::test_used_pages(kind), used + 1);
    drop(space);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(backing_budget::test_used_pages(kind), used);
}

fn test_rejected_release(kind: Kind, platform: bool) {
    let used = backing_budget::test_used_pages(kind);
    let ordinary = backing_budget::test_ordinary_pages(kind);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let owner = loader::create_user_address_space_handle();
    let rejected;
    {
        let mut table = ADDRESS_SPACE_TABLE.lock();
        let space = table.get_mut(owner.id()).unwrap();
        if platform {
            account(space, kind).mark_platform();
        }
        account(space, kind).set_limit(1).unwrap();
        let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let mut preparation = PreparingUserBacking::new(space, kind).unwrap();
        rejected = preparation.frame.as_ref().unwrap().frame();
        let mut attempts = 0;
        preparation.rollback_with(|frame| {
            assert_eq!(frame, rejected);
            attempts += 1;
            crate::logln!(
                "[backing preparation test] injecting {:?} release rejection platform={}",
                kind,
                platform
            );
            Err(physical::Error::CannotDeallocateUnallocatedFrame)
        });
        drop(preparation);
        assert_eq!(attempts, 1);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before - 1);
        assert_eq!(account(space, kind).pages(), 1);
        assert!(!space.is_mapped(VAddr::from(charlotte_launch::HEAP_VADDR)).unwrap());
        assert!(matches!(
            PreparingUserBacking::new(space, kind),
            Err(BackingPreparationError::Admission)
        ));
        account(space, kind).set_limit(2).unwrap();
        // Root destruction must refund this owned page but not rejected backing.
        PreparingUserBacking::new(space, kind)
            .unwrap()
            .map_with(
                VAddr::from(charlotte_launch::HEAP_VADDR),
                PageType::UserData,
                |space, mapping| space.map_existing_page(mapping).is_ok(),
            )
            .unwrap();
        assert_eq!(account(space, kind).pages(), 2);
        assert!(matches!(
            PreparingUserBacking::new(space, kind),
            Err(BackingPreparationError::Admission)
        ));
    }
    assert_eq!(backing_budget::test_used_pages(kind), used + 2);
    memory::close_user_address_space_handle(owner).unwrap();
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 1);
    assert_eq!(backing_budget::test_used_pages(kind), used + 1);
    let retained_ordinary = ordinary + u64::from(!platform);
    assert_eq!(backing_budget::test_ordinary_pages(kind), retained_ordinary);

    let replacement = loader::create_user_address_space_handle();
    assert_eq!(replacement.id(), owner.id());
    assert_ne!(replacement, owner);
    {
        let mut table = ADDRESS_SPACE_TABLE.lock();
        let space = table.get_mut(replacement.id()).unwrap();
        assert_eq!(account(space, kind).pages(), 0);
        let frame = PreparingUserBacking::new(space, kind)
            .unwrap()
            .map_with(
                VAddr::from(charlotte_launch::HEAP_VADDR),
                PageType::UserData,
                |space, mapping| space.map_existing_page(mapping).is_ok(),
            )
            .unwrap();
        assert_ne!(frame, rejected, "quarantined frame reused by replacement");
    }
    memory::close_user_address_space_handle(replacement).unwrap();
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 1);
    assert_eq!(backing_budget::test_used_pages(kind), used + 1);
    assert_eq!(backing_budget::test_ordinary_pages(kind), retained_ordinary);
}

fn test_unconfirmed_publication(kind: Kind, phase: usize) {
    let used = backing_budget::test_used_pages(kind);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let owner = loader::create_user_address_space_handle();
    {
        let mut table = ADDRESS_SPACE_TABLE.lock();
        let space = table.get_mut(owner.id()).unwrap();
        let mut preparation = PreparingUserBacking::new(space, kind).unwrap();
        let frame = preparation.frame.as_ref().unwrap().frame();
        preparation.may_be_published = true;
        preparation
            .space
            .map_existing_page(MemoryMapping {
                vaddr: VAddr::from(charlotte_launch::HEAP_VADDR),
                paddr: frame,
                page_type: PageType::UserData,
            })
            .unwrap();
        if phase != 0 {
            account(preparation.space, kind).commit_prepared(preparation.charge.as_mut().unwrap());
            if phase == 2 {
                drop(preparation.charge.take());
            }
        }
        let mapped = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        // Simulate interruption before commit, after commit with an inert token
        // still retained, and after removal of that token before frame transfer.
        preparation.rollback_with(|_| panic!("published backing deallocated without invalidation"));
        drop(preparation);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), mapped);
        assert_eq!(
            space.translate_address(VAddr::from(charlotte_launch::HEAP_VADDR)).unwrap(),
            frame
        );
        assert_eq!(account(space, kind).pages(), 1);
    }
    memory::close_user_address_space_handle(owner).unwrap();
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 1);
    assert_eq!(backing_budget::test_used_pages(kind), used + 1);
}

fn test_abandoned_release(kind: Kind) {
    let used = backing_budget::test_used_pages(kind);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let owner = loader::create_user_address_space_handle();
    {
        let mut table = ADDRESS_SPACE_TABLE.lock();
        let space = table.get_mut(owner.id()).unwrap();
        let mut preparation = PreparingUserBacking::new(space, kind).unwrap();
        let receipt =
            account(preparation.space, kind).retire_provisional(preparation.charge.take().unwrap());
        preparation.frame.take().unwrap().quarantine();
        // No confirmation, retry or recovery bypass. The receipt defaults to
        // retained counts even though its ordinary Rust borrow ends here.
        drop(receipt);
        drop(preparation);
        assert_eq!(account(space, kind).pages(), 1);
    }
    memory::close_user_address_space_handle(owner).unwrap();
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 1);
    assert_eq!(backing_budget::test_used_pages(kind), used + 1);
}
