//! Architecture-shared single-mutator boot probes. The private architecture
//! adapter is the production destructor walk, not a second cleanup path.

use super::{
    AddressSpace,
    AddressSpaceInterface,
    PAddr,
    PHYSICAL_FRAME_ALLOCATOR,
    PreparingUserBacking,
    PreparingUserFrame,
    VAddr,
    backing_budget::{
        self,
        Kind,
    },
    linear::{
        MemoryMapping,
        PageType,
    },
    physical::Error,
};

type Deallocator<'a> = dyn FnMut(PAddr) -> Result<(), Error> + 'a;

pub(crate) fn run(mut destroy: impl FnMut(&mut AddressSpace, &mut Deallocator<'_>) -> usize) {
    // A borrowed snapshot never enters the owning destructor walk.
    let mut borrowed = AddressSpace::get_current();
    assert_eq!(destroy(&mut borrowed, &mut |_| panic!("borrowed root freed a frame")), 0);
    drop(borrowed);

    // Four private table frames, followed by the tracked heap/image frames.
    // Reject each position separately, then all six. Synthetic Err happens
    // before the real allocator: rejected frames remain allocated forever.
    for reject in [0, 1, 2, 3, 4, 5, 6, usize::MAX] {
        let heap_before = backing_budget::test_used_pages(Kind::Heap);
        let image_before = backing_budget::test_used_pages(Kind::Image);
        let free_before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let mut space = AddressSpace::try_new_user().unwrap();
        install(&mut space, Kind::Heap, charlotte_launch::HEAP_VADDR);
        install(&mut space, Kind::Image, charlotte_launch::HEAP_VADDR + 4096);
        // An unowned leaf shares these tables, but not their backing lifetime.
        let foreign = PreparingUserFrame::allocate_zeroed().unwrap();
        space
            .map_existing_page(MemoryMapping {
                vaddr: VAddr::from(charlotte_launch::HEAP_VADDR + 8192),
                paddr: foreign.frame(),
                page_type: PageType::UserData,
            })
            .unwrap();
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free_before - 7);
        let failed = if reject == usize::MAX {
            6
        } else {
            usize::from(reject != 0)
        };
        let mut attempts = 0;
        assert_eq!(
            destroy(&mut space, &mut |frame| {
                assert_ne!(frame, foreign.frame(), "foreign leaf entered owning release");
                attempts += 1;
                if attempts == reject || reject == usize::MAX {
                    crate::logln!(
                        "[root release test] injecting rejection at release {} (all={})",
                        attempts,
                        reject == usize::MAX
                    );
                    Err(Error::CannotDeallocateUnallocatedFrame)
                } else {
                    PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame)
                }
            }),
            failed
        );
        assert_eq!(attempts, 6);
        assert!(!space.heap_account.accepting());
        assert!(!space.image_account.accepting());
        // Even success refunds only when the owning account fields drop.
        assert_eq!(backing_budget::test_used_pages(Kind::Heap), heap_before + 1);
        assert_eq!(backing_budget::test_used_pages(Kind::Image), image_before + 1);
        assert_eq!(destroy(&mut space, &mut |_| panic!("partial destruction retried a frame")), 0);
        drop(space);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free_before - failed - 1);
        let retained = u64::from(failed != 0);
        assert_eq!(backing_budget::test_used_pages(Kind::Heap), heap_before + retained);
        assert_eq!(backing_budget::test_used_pages(Kind::Image), image_before + retained);
        drop(foreign);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free_before - failed);
    }
    crate::logln!(
        "[root release] success, each table/root/heap/image failure, all-release failure, no \
         retry, borrowed root and foreign leaf passed (12 retained frames; 7 heap and 7 image \
         charges)"
    );
}

fn install(space: &mut AddressSpace, kind: Kind, vaddr: usize) {
    PreparingUserBacking::new(space, kind)
        .unwrap()
        .map_with(VAddr::from(vaddr), PageType::UserData, |space, mapping| {
            space.map_existing_page(mapping).is_ok()
        })
        .unwrap();
}
