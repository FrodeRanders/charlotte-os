//! # Initialization Module
use alloc::boxed::Box;

use crate::cpu::isa::init::IsaInitializer;
use crate::cpu::isa::interface::init::InitInterface;
use crate::cpu::isa::lp;
use crate::cpu::isa::lp::ops::get_lp_id;
use crate::cpu::scheduler::lp_schedulers::round_robin::RoundRobin;
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::klib::time::duration::ExtDuration;
use crate::memory::PHYSICAL_FRAME_ALLOCATOR;
use crate::memory::allocators::global_allocator::init_primary_allocator;
use crate::{log, logln};

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
    // Record the BSP's APIC ID now that the heap allocator is ready (BTreeMap requires the heap).
    #[cfg(target_arch = "x86_64")]
    crate::cpu::isa::interrupts::x2apic::X2Apic::record_id();
    // Pre-create all LP schedulers in LP ID order (0..lp_count) while single-threaded.
    // This ensures lp_schedulers[i] is always LP i's scheduler, regardless of AP init order.
    logln!("LP 0: Initializing LP local scheduler.");
    logln!("LP 0: Constructing LP scheduler.");
    let sched = Box::new(RoundRobin::new(get_lp_id(), ExtDuration::from_millis(10)));
    logln!("LP 0: Created new LP scheduler on the heap. Submitting it to the system scheduler.");
    unsafe {
        SYSTEM_SCHEDULER.write().set_lp_scheduler(sched);
    }
    logln!("LP 0: Submitted LP scheduler to the system scheduler.");
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
    unsafe {
        SYSTEM_SCHEDULER
            .write()
            .set_lp_scheduler(Box::new(RoundRobin::new(get_lp_id(), ExtDuration::from_millis(10))));
    }
    logln!("LP {lp_id}: ISA independent initialization complete.");
}
