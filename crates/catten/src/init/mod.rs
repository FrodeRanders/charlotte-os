//! # Initialization Module
use alloc::boxed::Box;

use crate::cpu::isa::init::IsaInitializer;
use crate::cpu::isa::interface::init::InitInterface;
use crate::cpu::isa::lp;
use crate::cpu::scheduler::lp_schedulers::round_robin::RoundRobin;
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::logln;
use crate::memory::PHYSICAL_FRAME_ALLOCATOR;
use crate::memory::allocators::global_allocator::init_primary_allocator;

pub fn bsp_init() {
    logln!("LP 0: Performing ISA specific initialization...");
    match IsaInitializer::init_bsp() {
        Ok(_) => logln!("LP 0: ISA specific initialization complete."),
        Err(e) => {
            // initialization failure is irrecoverable
            panic!("LP 0: ISA specific initialization failed: {e:?}");
        }
    }
    logln!("LP 0: Performing ISA independent initialization...");
    logln!("LP 0: Initializing physical memory...");
    match PHYSICAL_FRAME_ALLOCATOR.try_lock() {
        Some(pfa) => {
            logln!("LP 0: PhysicalFrameAllocator: {pfa:?}");
        }
        None => {
            panic!("LP 0: Failed to acquire lock on PhysicalFrameAllocator.");
        }
    }
    logln!("LP 0: Initializing kernel allocator...");
    init_primary_allocator();
    logln!("LP 0: Intialized kernel allocator.");
    logln!("LP 0: Initializing local scheduler...");
    let local_sched = Box::new(RoundRobin::default());
    logln!("LP 0: Local scheduler created, passing it to the system scheduler.");
    unsafe {
        SYSTEM_SCHEDULER.write().set_lp_scheduler(local_sched);
    }
    logln!("LP 0: Local scheduler initialized.");
    logln!("LP 0: ISA independent initialization complete.");
    logln!("LP 0: BSP initialization complete.");
}

pub fn ap_init() {
    let lp_id = lp::ops::get_lp_id();
    logln!("Initializing LP {lp_id}...");
    logln!("LP {lp_id}: Performing ISA specific initialization...");
    match IsaInitializer::init_ap() {
        Ok(_) => logln!("LP {lp_id}: ISA specific initialization complete."),
        Err(e) => {
            // initialization failure is irrecoverable
            panic!("LP {lp_id}: ISA specific initialization failed: {e:?}");
        }
    }
    logln!("LP {lp_id}: Performing ISA independent initialization.");
    logln!("LP {lp_id}: Initializing local scheduler...");
    let local_sched = Box::new(RoundRobin::default());
    unsafe {
        SYSTEM_SCHEDULER.write().set_lp_scheduler(local_sched);
    }
    logln!("LP {lp_id}: Local scheduler initialized.");
    logln!("LP {lp_id}: ISA independent initialization complete.");
}
