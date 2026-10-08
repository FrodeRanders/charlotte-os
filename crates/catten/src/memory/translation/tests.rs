//! Real private-tree construction with a per-walker allocation gate. No global
//! allocator fault mode, live-root installation or recovery of quarantined data.

use super::*;
use crate::{
    cpu::isa::memory::Error,
    memory::{
        AddressSpace,
        AddressSpaceInterface,
        PHYSICAL_FRAME_ALLOCATOR,
        VAddr,
        backing_budget::{
            self,
            Kind,
        },
        linear::{
            MemoryMapping,
            PageType,
        },
    },
};

pub(crate) fn run(
    mut map: impl FnMut(&mut AddressSpace, VAddr, PAddr, usize) -> Result<(), Error>,
) {
    PreparingTable::test_policy();
    super::account::test_pool();
    for scope in [TableScope::PrivateUser, TableScope::SharedKernel] {
        let mut account = super::Account::new();
        let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let charged = super::account::test_used_pages();
        let shared = super::shared::used_pages();
        let table = PreparingTable::allocate(
            scope,
            (scope == TableScope::PrivateUser).then_some(&mut account),
        )
        .unwrap();
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 1);
        let bytes: *const u8 = table.frame().into();
        assert!(unsafe { core::slice::from_raw_parts(bytes, 4096) }.iter().all(|&byte| byte == 0));
        table.cancel_unpublished().unwrap();
        assert_eq!(super::account::test_used_pages(), charged);
        assert_eq!(super::shared::used_pages(), shared);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    }
    #[cfg(target_arch = "aarch64")]
    let initial_tables = 0;
    #[cfg(target_arch = "x86_64")]
    let initial_tables = 1;
    let needed = 4 - initial_tables;
    for accepted in 0..=4 {
        let active = active_root();
        let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let heap = backing_budget::test_used_pages(Kind::Heap);
        let image = backing_budget::test_used_pages(Kind::Image);
        let tables = super::account::test_used_pages();
        let mut space = AddressSpace::try_new_user().unwrap();
        let data = PreparingUserFrame::allocate_zeroed().unwrap();
        let bytes: *mut u8 = data.frame().into();
        unsafe {
            core::ptr::write_bytes(bytes, 0x5a, 4096);
        }
        let first = VAddr::from(charlotte_launch::HEAP_VADDR);
        let result = map(&mut space, first, data.frame(), accepted);
        if accepted < needed {
            assert!(matches!(
                result,
                Err(Error::PMemError(crate::memory::physical::Error::OutOfFrames))
            ));
            assert!(!space.is_mapped(first).unwrap());
            assert_eq!(
                PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(),
                free - 1 - initial_tables - accepted
            );
            #[cfg(target_arch = "aarch64")]
            if accepted == 0 {
                assert_eq!(space.hw_asid(), 0);
            }
            space
                .map_existing_page(MemoryMapping {
                    vaddr: first,
                    paddr: data.frame(),
                    page_type: PageType::UserData,
                })
                .unwrap();
        } else {
            result.unwrap();
        }
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 5);
        assert_eq!(space.table_account.pages(), 4);
        assert_eq!(space.translate_address(first).unwrap(), data.frame());
        let leaf = space.unmap_page(first).unwrap();
        assert_eq!(leaf, data.frame());
        // Empty cached tables can remap even when the gate rejects all fresh
        // allocation. No table-count refund occurred at unmap.
        map(&mut space, first, data.frame(), 0).unwrap();
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 5);

        let sparse = VAddr::from(0x4000_0000usize);
        assert!(matches!(
            map(&mut space, sparse, data.frame(), 1),
            Err(Error::PMemError(crate::memory::physical::Error::OutOfFrames))
        ));
        assert!(!space.is_mapped(sparse).unwrap());
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 6);
        map(&mut space, sparse, data.frame(), 1).unwrap();
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 7);
        assert_eq!(space.table_account.pages(), 6);
        assert_eq!(space.translate_address(sparse).unwrap(), data.frame());
        assert!(
            unsafe { core::slice::from_raw_parts(bytes, 4096) }.iter().all(|&byte| byte == 0x5a)
        );
        assert_eq!(active_root(), active, "construction changed the active hardware roots");
        assert_inactive_root(&space);
        drop(space);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 1);
        data.release().unwrap();
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
        assert_eq!(backing_budget::test_used_pages(Kind::Heap), heap);
        assert_eq!(backing_budget::test_used_pages(Kind::Image), image);
        assert_eq!(super::account::test_used_pages(), tables);
    }
    crate::logln!(
        "[table preparation] scope/floor policy, zeroed-owner cancellation, every construction \
         prefix, retry/cached reuse, sparse partial tree and exact teardown passed (no retained \
         frames)"
    );
    raw_frame_abandonment();
    super::admission_tests::run();
    super::shared_tests::run();
}

fn active_root() -> (u64, u64) {
    let current = AddressSpace::get_current();
    #[cfg(target_arch = "aarch64")]
    return (current.get_ttbr0(), current.get_ttbr1());
    #[cfg(target_arch = "x86_64")]
    return (current.get_cr3(), 0);
}

fn assert_inactive_root(space: &AddressSpace) {
    let current = AddressSpace::get_current();
    #[cfg(target_arch = "aarch64")]
    assert_ne!(
        current.get_ttbr0(),
        space.get_ttbr0(),
        "inactive user root installed by construction"
    );
    #[cfg(target_arch = "x86_64")]
    assert_ne!(current.get_cr3(), space.get_cr3(), "inactive user root installed by construction");
}

fn raw_frame_abandonment() {
    // Initialize both pools before taking the physical allocator. These probes
    // never install a leaf or recover the frame left by a rejected release.
    let _ = account::test_used_pages();
    let _ = shared::used_pages();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let frame = PreparingUserFrame::allocate_zeroed().unwrap();
    account::test_with_pool_locked(|| {
        shared::test_with_pool_locked(|| {
            let _kernel = crate::memory::KERNEL_AS.lock();
            let _table = crate::memory::ADDRESS_SPACE_TABLE.lock();
            let physical = PHYSICAL_FRAME_ALLOCATOR.lock();
            drop(frame);
            assert_eq!(physical.free_frames(), free - 1);
        })
    });
    let frame = PreparingUserFrame::allocate_zeroed().unwrap();
    let mut attempts = 0;
    assert!(matches!(
        frame.release_with(|_| {
            attempts += 1;
            Err(crate::memory::physical::Error::CannotDeallocateUnallocatedFrame)
        }),
        Err(crate::memory::physical::Error::CannotDeallocateUnallocatedFrame)
    ));
    assert_eq!(attempts, 1);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 2);
    crate::logln!(
        "[raw frame preparation] Drop under allocator/table/pool guards and terminal release \
         rejection passed (two retained frames)"
    );
}
