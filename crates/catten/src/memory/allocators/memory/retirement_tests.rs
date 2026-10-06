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
    crate::cpu::isa::memory::tlb::inval_range_kernel(second, 1);
    drop(foreign);
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
    // Abandonment is deliberately fail-closed. This fixture permanently
    // reserves one 4 KiB page; it must never be adopted/freed a second time.
    let quarantined = QUARANTINED_KERNEL_PAGES.load(Ordering::Relaxed);
    try_allocate_and_map_range(base, PageSize::Standard, 1, &mut retirement).unwrap();
    retire_kernel_range(base, PageSize::Standard, 1, &mut retirement).unwrap();
    assert_eq!(free(), baseline - 1);
    drop(retirement);
    assert_eq!(free(), baseline - 1);
    assert_eq!(QUARANTINED_KERNEL_PAGES.load(Ordering::Relaxed), quarantined + 1);
    crate::logln!(
        "[kernel retirement] detach-before-release, failed barrier/retry, allocation/map \
         rollback, foreign-leaf preservation, bounded metadata and Drop quarantine (one reserved \
         test page) passed"
    );
}
