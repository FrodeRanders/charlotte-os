//! Single-mutator boot fixtures; barriers use real local/broadcast invalidation.

use super::*;
use crate::memory::linear::address_map::{
    LA_MAP,
    RegionType,
};

fn free() -> usize {
    PHYSICAL_FRAME_ALLOCATOR.lock().free_frames()
}

pub(crate) fn test_kernel_retirement() {
    let base = LA_MAP.get_region(RegionType::KernelStackArena).base + 16usize * 1024 * 1024;
    let mut retirement = RetiredKernelRange::new();
    // Warm the retained kernel table subtree before comparing data counts.
    try_allocate_and_map_range(base, PageSize::Standard, 2, &mut retirement).unwrap();
    retire_kernel_range(base, PageSize::Standard, 2, &mut retirement).unwrap();
    retirement.release().unwrap();
    let baseline = free();

    try_allocate_and_map_range(base, PageSize::Standard, 2, &mut retirement).unwrap();
    assert_eq!(free(), baseline - 2);
    retire_kernel_range(base, PageSize::Standard, 2, &mut retirement).unwrap();
    assert!(!KERNEL_AS.lock().is_mapped(base).unwrap());
    assert!(!KERNEL_AS.lock().is_mapped(base + AddressSpace::PAGE_SIZE).unwrap());
    assert_eq!(free(), baseline - 2);
    assert!(matches!(retirement.release_with(|_, _| false), Err(Error::RetirementFailed)));
    assert_eq!(free(), baseline - 2);
    assert!(!retirement.quiescent);
    assert!(KERNEL_AS.try_lock().is_some());
    retirement.release().unwrap();
    assert_eq!(free(), baseline);
    retirement.release().unwrap(); // Retry after success cannot release twice.
    assert_eq!(free(), baseline);

    // Allocation and mapping failures after an installed prefix both retain
    // its data until post-guard cleanup. The uninstalled frame has its owner.
    let result = allocate_and_map_with(
        base,
        PageSize::Standard,
        2,
        &mut retirement,
        |size, index| {
            if index == 1 {
                Err(Error::PfaError(physical::Error::OutOfFrames))
            } else {
                size.allocate()
            }
        },
        |space, mapping, _| space.map_page(mapping),
    );
    assert!(matches!(result, Err(Error::PfaError(physical::Error::OutOfFrames))));
    assert!(!KERNEL_AS.lock().is_mapped(base).unwrap());
    assert_eq!(free(), baseline - 1);
    retirement.release().unwrap();
    assert_eq!(free(), baseline);

    let result = allocate_and_map_with(
        base,
        PageSize::Standard,
        2,
        &mut retirement,
        |size, _| size.allocate(),
        |space, mapping, index| {
            if index == 1 {
                Err(crate::cpu::isa::memory::Error::AlreadyMapped)
            } else {
                space.map_page(mapping)
            }
        },
    );
    assert!(matches!(result, Err(Error::IsaMemoryError(_))));
    assert!(!KERNEL_AS.lock().is_mapped(base).unwrap());
    assert_eq!(free(), baseline - 2);
    retirement.release().unwrap();
    assert_eq!(free(), baseline);

    // Real AlreadyMapped on the second leaf must not steal foreign backing.
    let foreign = PageSize::Standard.allocate().unwrap();
    let second = base + AddressSpace::PAGE_SIZE;
    KERNEL_AS
        .lock()
        .map_page(MemoryMapping {
            vaddr: second,
            paddr: foreign.frame(),
            page_type: PageType::KernelData,
        })
        .unwrap();
    assert!(matches!(
        try_allocate_and_map_range(base, PageSize::Standard, 2, &mut retirement),
        Err(Error::IsaMemoryError(_))
    ));
    assert_eq!(KERNEL_AS.lock().translate_address(second).unwrap(), foreign.frame());
    assert_eq!(free(), baseline - 3);
    retirement.release().unwrap();
    assert_eq!(free(), baseline - 1);
    assert_eq!(KERNEL_AS.lock().unmap_page(second).unwrap(), foreign.frame());
    let mut foreign_retirement = RetiredKernelRange::new();
    foreign_retirement.begin(second, PageSize::Standard, 1).unwrap();
    foreign.retire(&mut foreign_retirement);
    foreign_retirement.release().unwrap();
    assert_eq!(free(), baseline);

    // Admission bounds precede mutation; the progress receipt can be reused.
    assert!(
        try_allocate_and_map_range(
            base,
            PageSize::Standard,
            KERNEL_RANGE_FRAME_CAPACITY + 1,
            &mut retirement
        )
        .is_err()
    );
    assert!(
        try_allocate_and_map_range(base + 1usize, PageSize::Standard, 1, &mut retirement).is_err()
    );
    assert_eq!(free(), baseline);
    assert!(!KERNEL_AS.lock().is_mapped(base).unwrap());
    assert_eq!(retirement.len, 0);
    test_physical_release(base);
    let baseline = free();
    // Abandonment is deliberately fail-closed. This fixture permanently
    // reserves one 4 KiB page; it must never be adopted/freed a second time.
    let quarantined = QUARANTINED_KERNEL_PAGES.load(Ordering::Relaxed);
    try_allocate_and_map_range(base, PageSize::Standard, 1, &mut retirement).unwrap();
    retire_kernel_range(base, PageSize::Standard, 1, &mut retirement).unwrap();
    assert_eq!(free(), baseline - 1);
    {
        let _space = KERNEL_AS.lock();
        let allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        drop(retirement);
        assert_eq!(allocator.free_frames(), baseline - 1);
    }
    assert_eq!(free(), baseline - 1);
    assert_eq!(QUARANTINED_KERNEL_PAGES.load(Ordering::Relaxed), quarantined + 1);
    test_preparation_abandonment(base);
    crate::logln!(
        "[kernel retirement] detach-before-release, failed barrier/retry, allocation/map \
         rollback, foreign-leaf preservation, bounded metadata and Drop quarantine under \
         allocator/table guards (one reserved test page) and preparation abandonment passed"
    );
}

fn invalidate(base: VAddr, pages: usize) -> bool {
    crate::cpu::isa::memory::tlb::try_inval_range_kernel(base, pages).is_ok()
}

fn guards_available() {
    assert!(PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some());
    assert!(KERNEL_AS.try_lock().is_some());
}

fn test_physical_release(base: VAddr) {
    // Cross receipt entries and exercise a short final batch. No allocator
    // guard survives into the between-batch hook or another subsystem.
    let mut retirement = RetiredKernelRange::new();
    let baseline = free();
    try_allocate_and_map_range(base, PageSize::Standard, 35, &mut retirement).unwrap();
    retire_kernel_range(base, PageSize::Standard, 35, &mut retirement).unwrap();
    let mut batches = 0;
    let mut boundaries = 0;
    retirement
        .release_with_batches(
            invalidate,
            |frames| {
                guards_available();
                batches += 1;
                assert_eq!(
                    frames.len(),
                    if batches == 3 {
                        3
                    } else {
                        16
                    }
                );
                release_batch(frames)
            },
            || {
                guards_available();
                boundaries += 1;
            },
        )
        .unwrap();
    assert_eq!((batches, boundaries), (3, 3));
    assert_eq!(free(), baseline);

    // A real 2 MiB leaf must be detached and invalidated before any of its
    // 512 base frames are returned. Warm any architecture table metadata.
    let heap_probe = physical::HEAP_PHYS_BASE.load(Ordering::Relaxed);
    let large_base = base + PageSize::Large.num_bytes();
    try_allocate_and_map_range(large_base, PageSize::Large, 1, &mut retirement).unwrap();
    retire_kernel_range(large_base, PageSize::Large, 1, &mut retirement).unwrap();
    let mut batches = 0;
    retirement
        .release_with_batches(
            invalidate,
            |frames| {
                batches += 1;
                assert_eq!(frames.len(), RELEASE_BATCH_PAGES);
                release_batch(frames)
            },
            guards_available,
        )
        .unwrap();
    assert_eq!(batches, 32);
    let baseline = free();
    let quarantined = QUARANTINED_KERNEL_PAGES.load(Ordering::Relaxed);
    try_allocate_and_map_range(large_base, PageSize::Large, 1, &mut retirement).unwrap();
    let first = KERNEL_AS.lock().translate_address(large_base).unwrap();
    retire_kernel_range(large_base, PageSize::Large, 1, &mut retirement).unwrap();
    assert!(!KERNEL_AS.lock().is_mapped(large_base).unwrap());
    let mut batches = 0;
    assert!(matches!(
        retirement.release_with_batches(
            invalidate,
            |frames| {
                batches += 1;
                if batches == 32 {
                    let (completed, result) = release_batch(&frames[..15]);
                    result.unwrap();
                    // Reject before the last real deallocation. Its backing
                    // remains unavailable; earlier frames are genuinely free.
                    (completed, Err(physical::Error::InvalidPAddr))
                } else {
                    release_batch(frames)
                }
            },
            guards_available,
        ),
        Err(Error::PfaError(physical::Error::InvalidPAddr))
    ));
    assert_eq!(batches, 32);
    assert_eq!(retirement.released_pages, 511);
    assert!(retirement.release_started);
    guards_available();
    assert_eq!(free(), baseline - 1);
    // Kernel physical-ownership fixture: reclaim the first freed address for
    // a new owner to reproduce allocator reuse, then forbid stale cleanup.
    PHYSICAL_FRAME_ALLOCATOR.lock().mark_frame_unavailable(first).unwrap();
    let successor = PreparingKernelFrame {
        frame: Some(first),
        page_size: PageSize::Standard,
    };
    assert!(matches!(
        retirement.release_with_batches(
            |_, _| panic!("frozen receipt retried invalidation"),
            |_| panic!("frozen receipt touched successor backing"),
            || panic!("frozen receipt ran a completion hook"),
        ),
        Err(Error::RetirementFailed)
    ));
    assert!(matches!(retirement.begin(base, PageSize::Standard, 0), Err(Error::InvalidRange)));
    assert_eq!(free(), baseline - 2);
    drop(retirement);
    assert_eq!(QUARANTINED_KERNEL_PAGES.load(Ordering::Relaxed), quarantined + 1);
    assert_eq!(free(), baseline - 2);
    let mut successor_retirement = RetiredKernelRange::new();
    successor_retirement.begin(base, PageSize::Standard, 1).unwrap();
    successor.retire(&mut successor_retirement);
    successor_retirement.release().unwrap();
    assert_eq!(free(), baseline - 1);
    // Large allocation updates a legacy boot-heap diagnostic probe. This
    // serialized fixture must not leave it pointing at reclaimed test data.
    physical::HEAP_PHYS_BASE.store(heap_probe, Ordering::Relaxed);

    // Simulate interruption after the real barrier and phase admission but
    // before its first physical callback. There is no synthetic hardware ACK.
    let mut interrupted = RetiredKernelRange::new();
    try_allocate_and_map_range(base, PageSize::Standard, 1, &mut interrupted).unwrap();
    retire_kernel_range(base, PageSize::Standard, 1, &mut interrupted).unwrap();
    assert!(invalidate(base, 1));
    interrupted.quiescent = true;
    interrupted.release_started = true;
    assert!(matches!(interrupted.release(), Err(Error::RetirementFailed)));
    drop(interrupted);
    assert_eq!(free(), baseline - 2);
    assert_eq!(QUARANTINED_KERNEL_PAGES.load(Ordering::Relaxed), quarantined + 2);
    crate::logln!(
        "[kernel physical release] 35-page/2 MiB batches, post-guard completion, terminal partial \
         failure, successor preservation and interrupted owner passed (two additional retained \
         base frames)"
    );
}

fn test_preparation_abandonment(base: VAddr) {
    let baseline = free();
    let quarantined = QUARANTINED_KERNEL_PAGES.load(Ordering::Relaxed);
    // Even a definitely unpublished owner cannot infer whether its caller
    // holds the physical allocator. Drop must not acquire it or release data.
    let unpublished = PageSize::Standard.allocate().unwrap();
    {
        let allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        assert_eq!(allocator.free_frames(), baseline - 1);
        drop(unpublished);
        assert_eq!(allocator.free_frames(), baseline - 1);
    }
    assert_eq!(QUARANTINED_KERNEL_PAGES.load(Ordering::Relaxed), quarantined + 1);

    // Simulate successful leaf publication before the preparation owner is
    // consumed. Both guards are live at abandonment, without panic unwinding
    // or manufactured TLB acknowledgements. Warmed tables allocate nothing.
    let published = PageSize::Standard.allocate().unwrap();
    let frame = published.frame();
    let address = base + 2 * AddressSpace::PAGE_SIZE;
    {
        let mut space = KERNEL_AS.lock();
        space
            .map_page(MemoryMapping {
                vaddr: address,
                paddr: frame,
                page_type: PageType::KernelData,
            })
            .unwrap();
        let allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        assert_eq!(allocator.free_frames(), baseline - 2);
        drop(published);
        assert_eq!(allocator.free_frames(), baseline - 2);
        assert_eq!(space.translate_address(address).unwrap(), frame);
        // Remove this fixture's leaf, but never re-adopt abandoned backing.
        assert_eq!(space.unmap_page(address).unwrap(), frame);
    }
    assert!(invalidate(address, 1));
    assert_eq!(free(), baseline - 2);
    assert_eq!(QUARANTINED_KERNEL_PAGES.load(Ordering::Relaxed), quarantined + 2);
    guards_available();
    crate::logln!(
        "[kernel preparation] unpublished/published abandonment retains backing under \
         allocator/table guards (two additional reserved pages)"
    );
}
