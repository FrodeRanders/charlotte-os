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
