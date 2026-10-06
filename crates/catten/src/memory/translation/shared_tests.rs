//! Single-mutator boot fixtures; linked kernel branches remain kernel-owned.
use super::*;
use crate::memory::{
    AddressSpace,
    AddressSpaceInterface,
    KERNEL_AS,
    PHYSICAL_FRAME_ALLOCATOR,
    VAddr,
    linear::{
        MemoryMapping,
        PageType,
        address_map::{
            LA_MAP,
            RegionType,
        },
    },
};

pub(super) fn run() {
    preparation();
    shared_tree();
    retained_preparation();
    crate::logln!(
        "[shared table admission] rejection before backing allocation, unused/zeroed-owner \
         refund, sparse partial-tree retention/retry, cached reuse at the ceiling and shared-root \
         counting passed; rejected/abandoned preparation retains two frames/charges"
    );
}

fn preparation() {
    let used = shared::used_pages();
    let quarantined = shared::quarantined_pages();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert!(PreparingTable::allocate_with(TableScope::SharedKernel, None, |_| None).is_none());
    assert_eq!(shared::used_pages(), used);
    let pressure = shared::Pressure::new(0);
    let mut allocated = false;
    assert!(
        PreparingTable::allocate_with(TableScope::SharedKernel, None, |_| {
            allocated = true;
            PreparingUserFrame::allocate_zeroed()
        })
        .is_none()
    );
    assert!(!allocated, "admission must precede physical allocation");
    assert_eq!(shared::used_pages(), used);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    drop(pressure);
    drop(PreparingTable::allocate(TableScope::SharedKernel, None).unwrap());
    assert_eq!(shared::used_pages(), used);
    assert_eq!(shared::quarantined_pages(), quarantined);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
}

fn map(space: &mut AddressSpace, vaddr: VAddr, frame: PAddr) -> bool {
    space
        .map_existing_page(MemoryMapping {
            vaddr,
            paddr: frame,
            page_type: PageType::KernelData,
        })
        .is_ok()
}

fn shared_tree() {
    // Isolated boot-only locations, within every supported kernel MMIO region.
    // No MMIO allocator reservation or physical device is adopted by this test.
    let region = LA_MAP.get_region(RegionType::KernelMmio);
    let first = region.base + 0x8000_0000usize;
    let sparse = first + 0x4000_0000usize;
    assert!(region.contains(first) && region.contains(sparse));
    let used = shared::used_pages();
    let quarantined = shared::quarantined_pages();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let data = PreparingUserFrame::allocate_zeroed().unwrap();
    let linked;
    {
        let mut kernel = KERNEL_AS.lock();
        assert!(!kernel.is_mapped(first).unwrap());
        assert!(!kernel.is_mapped(sparse).unwrap());
        assert!(map(&mut kernel, first, data.frame()));
        linked = shared::used_pages() - used;
        assert!(linked > 0);
        assert_eq!(kernel.unmap_page(first).unwrap(), data.frame());
    }
    // The source leaf is gone before any frame can be returned; empty linked
    // tables remain live and charged. Never invalidate under KERNEL_AS.
    crate::cpu::isa::memory::tlb::try_inval_range_kernel(first, 1).unwrap();
    let pressure = shared::Pressure::new(1);
    {
        let mut kernel = KERNEL_AS.lock();
        assert!(!map(&mut kernel, sparse, data.frame()));
        assert!(!kernel.is_mapped(sparse).unwrap());
        assert_eq!(shared::used_pages(), used + linked + 1);
        let at_limit = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        for _ in 0..16 {
            assert!(!map(&mut kernel, sparse, data.frame()));
            assert!(map(&mut kernel, first, data.frame()));
            assert_eq!(kernel.unmap_page(first).unwrap(), data.frame());
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), at_limit);
            assert_eq!(shared::used_pages(), used + linked + 1);
        }
    }
    crate::cpu::isa::memory::tlb::try_inval_range_kernel(first, 1).unwrap();
    drop(pressure);
    {
        let mut kernel = KERNEL_AS.lock();
        assert!(map(&mut kernel, sparse, data.frame()));
        assert_eq!(shared::used_pages(), used + linked + 2);
    }
    let charged = shared::used_pages();
    let private = account::test_used_pages();
    for _ in 0..4 {
        let mut root = AddressSpace::try_new_user().unwrap();
        assert_eq!(root.translate_address(sparse).unwrap(), data.frame());
        assert_eq!(shared::used_pages(), charged);
        drop(root);
        assert_eq!(shared::used_pages(), charged);
        assert_eq!(account::test_used_pages(), private);
    }
    {
        let mut kernel = KERNEL_AS.lock();
        assert_eq!(kernel.unmap_page(sparse).unwrap(), data.frame());
    }
    crate::cpu::isa::memory::tlb::try_inval_range_kernel(sparse, 1).unwrap();
    drop(data);
    assert_eq!(shared::used_pages(), charged);
    assert_eq!(shared::quarantined_pages(), quarantined);
    let retained: usize = (charged - used).try_into().unwrap();
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - retained);
}

fn retained_preparation() {
    for abandoned in [false, true] {
        let used = shared::used_pages();
        let quarantined = shared::quarantined_pages();
        let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let pressure = shared::Pressure::new(1);
        let mut preparation = PreparingTable::allocate(TableScope::SharedKernel, None).unwrap();
        if abandoned {
            // Simulate interruption after disarming the frame owner.
            preparation.state = PreparationState::Publishing;
            preparation.frame.take().unwrap().quarantine();
        } else {
            preparation.rollback_with(|_| {
                Err(crate::memory::physical::Error::CannotDeallocateUnallocatedFrame)
            });
        }
        drop(preparation);
        assert_eq!(shared::used_pages(), used + 1);
        assert_eq!(shared::quarantined_pages(), quarantined + 1);
        assert!(PreparingTable::allocate(TableScope::SharedKernel, None).is_none());
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 1);
        drop(pressure);
        // A successful provisional retry cannot refund the retained frame.
        drop(PreparingTable::allocate(TableScope::SharedKernel, None).unwrap());
        assert_eq!(shared::used_pages(), used + 1);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 1);
    }
}
