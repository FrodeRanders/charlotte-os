//! Bounded startup checks, independent of the optional regression suite.
//!
//! Run on the BSP before AP admission. Check the resources this boot actually
//! needs without fault injection, synthetic domains or persistent test workers.

use alloc::alloc::{
    alloc,
    dealloc,
};
use core::{
    alloc::Layout,
    sync::atomic::Ordering,
};

use crate::{
    cpu::isa::interface::memory::address::PhysicalAddress,
    memory::{
        PHYSICAL_FRAME_ALLOCATOR,
        allocators::global_allocator::{
            HEAP_GROWTH_RESERVE_BYTES,
            MAX_HEAP_GROWTH_RESERVE,
            MIN_HEAP_GROWTH_RESERVE,
        },
    },
};

#[cfg(feature = "kernel_tests")]
pub const MODE: &str = "CHARLOTTE_BOOT_MODE=tests";
#[cfg(not(feature = "kernel_tests"))]
pub const MODE: &str = "CHARLOTTE_BOOT_MODE=minimal";

pub fn validate_memory() {
    crate::logln!("{MODE}");
    let (usable, free) = {
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        let usable = allocator.usable_bytes();
        let free = allocator.free_frames();
        assert!(free > 0 && free as u64 <= usable / 4096, "boot memory unavailable or invalid");
        let frame = allocator.allocate_frame().expect("boot memory frame unavailable");
        assert_eq!(allocator.free_frames(), free - 1, "boot frame accounting mismatch");
        // This frame is private and never mapped or published. Volatile access
        // makes this a real readback rather than an optimized constant check.
        unsafe {
            let pointer = frame.into_hhdm_mut::<u64>();
            pointer.write_volatile(0xcafe_babe_1234_5678);
            pointer.add(511).write_volatile(0x8765_4321_dead_beef);
            assert_eq!(pointer.read_volatile(), 0xcafe_babe_1234_5678);
            assert_eq!(pointer.add(511).read_volatile(), 0x8765_4321_dead_beef);
        }
        allocator.deallocate_frame(frame).expect("boot frame release failed");
        assert_eq!(allocator.free_frames(), free, "boot frame release accounting mismatch");
        (usable, free)
    };
    let reserve = HEAP_GROWTH_RESERVE_BYTES.load(Ordering::Relaxed);
    assert!((MIN_HEAP_GROWTH_RESERVE..=MAX_HEAP_GROWTH_RESERVE).contains(&reserve));
    assert_eq!(reserve % (2 * 1024 * 1024), 0, "boot heap reserve alignment");

    // One bounded, aligned allocation replaces the old verbose 1050-byte and
    // 8 KiB exercises. Larger/failing retirement paths belong to kernel_tests.
    let layout = Layout::from_size_align(8192, 64).unwrap();
    let pointer = unsafe { alloc(layout) };
    assert!(!pointer.is_null(), "boot heap allocation unavailable");
    assert_eq!(pointer as usize % layout.align(), 0, "boot heap alignment mismatch");
    unsafe {
        for index in 0..layout.size() {
            pointer.add(index).write_volatile(index as u8);
        }
        for index in 0..layout.size() {
            assert_eq!(pointer.add(index).read_volatile(), index as u8, "boot heap readback");
        }
        dealloc(pointer, layout);
    }
    crate::logln!(
        "[boot validation] memory passed: usable={} MiB, free={} frames, heap reserve={} MiB",
        usable / (1024 * 1024),
        free,
        reserve / (1024 * 1024)
    );
}
