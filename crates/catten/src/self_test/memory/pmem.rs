use crate::{
    cpu::isa::interface::memory::address::PhysicalAddress,
    logln,
    memory::PHYSICAL_FRAME_ALLOCATOR,
};

pub fn test_pmem() {
    let mut pfa_lock = PHYSICAL_FRAME_ALLOCATOR.lock();

    match pfa_lock.allocate_frame() {
        Ok(ref frame) => {
            let magic_number = 0xcafebabeu32;
            unsafe {
                let frame_ptr = frame.into_hhdm_mut::<u32>();
                frame_ptr.write(magic_number);
                assert_eq!(frame_ptr.read(), magic_number, "Physical memory readback mismatch");
            }
            pfa_lock.deallocate_frame(*frame).expect("Failed to deallocate physical memory frame");
        }
        Err(e) => panic!("Failed to allocate a frame from the physical frame allocator: {:?}", e),
    }
    logln!("All physical memory subsystem tests passed.");
}
