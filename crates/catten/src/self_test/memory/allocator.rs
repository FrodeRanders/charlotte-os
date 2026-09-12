use alloc::alloc::{
    alloc,
    dealloc,
};
use core::{
    alloc::Layout,
    sync::atomic::Ordering,
};

use crate::{
    logln,
    memory::allocators::global_allocator::{
        HEAP_GROWTH_RESERVE_BYTES,
        MAX_HEAP_GROWTH_RESERVE,
        MIN_HEAP_GROWTH_RESERVE,
    },
};

pub fn test_allocator() {
    logln!("Starting the kernel allocator self-test...");
    let reserve = HEAP_GROWTH_RESERVE_BYTES.load(Ordering::Relaxed);
    assert!(
        (MIN_HEAP_GROWTH_RESERVE..=MAX_HEAP_GROWTH_RESERVE).contains(&reserve),
        "kernel heap growth reserve outside its configured bounds"
    );
    const LARGE_PAGE: usize = 2 * 1024 * 1024;
    assert_eq!(reserve % LARGE_PAGE, 0, "kernel heap growth reserve must be large-page aligned");
    logln!("Kernel allocator self-test: heap growth reserve is {} MiB", reserve / (1024 * 1024));

    // Resource-policy plumbing: a retired generation's recorded high-water
    // mark must survive domain teardown and feed the next launch decision.
    const POLICY_TEST_PRINCIPAL: u64 = u64::MAX;
    crate::memory::usage::remember_principal_stack_high_water(POLICY_TEST_PRINCIPAL, 7);
    assert_eq!(
        crate::memory::usage::principal_stack_high_water(POLICY_TEST_PRINCIPAL),
        7,
        "principal stack high-water was not retained"
    );
    assert_eq!(
        charlotte_lifecycle::adaptive_stack_pages(7, 4, 64),
        8,
        "adaptive stack policy lost its one-page headroom"
    );

    // The free-frame sensor must track allocation exactly, and pressure must
    // withhold growth above the default without ever shrinking below it.
    {
        use crate::memory::PHYSICAL_FRAME_ALLOCATOR;
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        let before = allocator.free_frames();
        let frame = allocator.allocate_frame().expect("free-frame accounting probe failed");
        assert_eq!(allocator.free_frames(), before - 1, "allocation must decrement free frames");
        allocator.deallocate_frame(frame).expect("free-frame accounting probe free failed");
        assert_eq!(allocator.free_frames(), before, "deallocation must restore free frames");
        let usable_frames = allocator.usable_bytes() / 4096;
        assert!((before as u64) <= usable_frames, "free frames cannot exceed usable frames");
        assert_eq!(
            charlotte_lifecycle::damp_stack_growth(8, 4, before as u64 - 1, before as u64),
            4,
            "memory pressure must withhold history-based stack growth"
        );
        assert_eq!(
            charlotte_lifecycle::damp_stack_growth(8, 4, before as u64, before as u64),
            8,
            "stack growth must resume at the reserve boundary"
        );
    }
    logln!("Kernel allocator self-test: adaptive stack policy plumbing verified");
    logln!("Kernel allocator self-test: Allocating 1050 bytes...");
    let layout_1050 = Layout::from_size_align(1050, 64).unwrap();
    let ptr = unsafe { alloc(layout_1050) };
    assert!(!ptr.is_null(), "Kernel allocator self-test: allocation of 1050 bytes failed");
    logln!("Kernel allocator self-test: Allocated 1050 bytes at {:p}", ptr);
    logln!("Kernel allocator self-test: Writing to allocated memory...");
    for i in 0..1050 {
        unsafe {
            ptr.add(i).write(i as u8);
        }
    }
    logln!("Kernel allocator self-test: Write complete.");
    logln!("Kernel allocator self-test: Reading from allocated memory...");
    for i in 0..1050 {
        assert_eq!(unsafe { ptr.add(i).read() }, i as u8);
    }
    logln!("Kernel allocator self-test: Read complete.");
    logln!("Kernel allocator self-test: Deallocating allocated memory...");
    unsafe {
        dealloc(ptr, layout_1050);
    }
    logln!("Kernel allocator self-test: Deallocation complete.");
    logln!("Kernel allocator self-test: Allocating 8 KiB...");
    let layout_8k = Layout::from_size_align(8192, 8).unwrap();
    let ptr = unsafe { alloc(layout_8k) };
    assert!(!ptr.is_null(), "Kernel allocator self-test: allocation of 8 KiB failed");
    logln!("Kernel allocator self-test: Allocated 8 KiB at {:p}", ptr);
    logln!("Kernel allocator self-test: Writing to allocated memory...");
    for i in 0..8192 {
        unsafe {
            ptr.add(i).write(i as u8);
        }
    }
    logln!("Kernel allocator self-test: Write complete.");
    logln!("Kernel allocator self-test: Reading from allocated memory...");
    for i in 0..8192 {
        assert_eq!(unsafe { ptr.add(i).read() }, i as u8);
    }
    logln!("Kernel allocator self-test: Read complete.");
    logln!("Kernel allocator self-test: Deallocating allocated memory...");
    unsafe {
        dealloc(ptr, layout_8k);
    }
    logln!("Kernel allocator self-test: Deallocation complete.");

    logln!("Kernel allocator self-test: PASSED");
}
