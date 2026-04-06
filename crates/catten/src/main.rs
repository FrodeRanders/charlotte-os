#![no_std]
#![no_main]
#![feature(abi_custom)]
#![feature(extend_one)]
#![feature(iter_advance_by)]
#![feature(likely_unlikely)]
#![feature(step_trait)]
#![allow(static_mut_refs)]
#![allow(named_asm_labels)]

//! # Catten
//!
//! Catten is an operating system kernel developed as a component of CharlotteOS, an
//! experimental modern operating system.It is responsible for initializing the hardware,
//! providing common abstractions for all hardware resources, and managing the execution of
//! user-space applications and the environment in which they run. It is a crucial part of the
//! operating system, as it provides the foundation on which the rest of the system is built and it
//! touches every hardware and software component of the system on which it is used. While it is
//! developed as a component of CharlotteOS, it is designed to be modular and flexible, and thus
//! useful in other operating systems, embedded firmware, and other types of software systems
//! as well.

extern crate alloc;

pub mod cpu;
pub mod drivers;
pub mod dwm;
pub mod environment;
pub mod framebuffer;
pub mod init;
pub mod klib;
pub mod log;
pub mod memory;
pub mod panic;
pub mod self_test;
pub mod timers;

use limine::mp::MpInfo;
use spin::{Barrier, Lazy};

use crate::cpu::isa::interface::interrupts::LocalIntCtlrIfce;
use crate::cpu::isa::interface::system_info::CpuInfoIfce;
use crate::cpu::isa::interrupts::LocalIntCtlr;
use crate::cpu::isa::lp::ops::{get_lp_id, yield_lp};
use crate::cpu::isa::system_info::CpuInfo;
use crate::cpu::isa::timers::print_timer_info;
use crate::cpu::multiprocessor::get_lp_count;
use crate::cpu::multiprocessor::startup::{assign_id, start_secondary_lps};
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::cpu::scheduler::threads::{MASTER_THREAD_TABLE, Thread, ThreadId};
use crate::memory::KERNEL_ASID;

const KERNEL_VERSION: (u64, u64, u64) = (0, 3, 5);
static INIT_BARRIER: Lazy<Barrier> = Lazy::new(|| Barrier::new(get_lp_count() as usize));
static YIELD_BARRIER: Lazy<Barrier> = Lazy::new(|| Barrier::new(get_lp_count() as usize));
/// This is the bootstrap processor's entry point into the kernel. The `bsp_main` function is
/// called by the bootloader after setting up the environment. It is made C ABI compatible so
/// that it can be called by Limine or any other Limine Boot Protocol compliant bootloader.
#[unsafe(no_mangle)]
pub extern "C" fn bsp_main() -> ! {
    logln!(
        "Catten Kernel Version {}.{}.{}",
        (KERNEL_VERSION.0),
        (KERNEL_VERSION.1),
        (KERNEL_VERSION.2)
    );
    logln!("========================================================================");
    logln!("Initializing the system using the bootstrap processor...");
    unsafe {
        assign_id();
    }
    logln!("BSP assigned ID 0.");
    init::bsp_init();
    logln!("System initialized.");
    logln!("Starting secondary LPs...");
    start_secondary_lps().expect("Failed to start secondary LPs");
    INIT_BARRIER.wait();
    self_test::run_self_tests();
    logln!("System Information:");
    logln!("CPU Vendor: {}", (CpuInfo::get_vendor()));
    logln!("CPU Model: {}", (CpuInfo::get_model()));
    logln!("Physical Address bits implemented: {}", (CpuInfo::get_paddr_sig_bits()));
    logln!("Virtual Address bits implemented: {}", (CpuInfo::get_vaddr_sig_bits()));
    print_timer_info();
    mask_interrupts!();
    for _ in 0..(get_lp_count() * 2) {
        logln!("Creating new thread.");
        let thread = Thread::new(false, KERNEL_ASID, test_fn as *const fn());
        logln!("Created thread.");
        let id = MASTER_THREAD_TABLE.write().add_element(thread);
        logln!("Added thread to master thread table with id = {id}.");
        SYSTEM_SCHEDULER
            .read()
            .submit_ready_thread(id as ThreadId)
            .expect("Error submitting ready thread to system scheduler");
        logln!("Submitted thread with ID = {id} to the system scheduler.");
    }
    unmask_interrupts!();
    logln!("Submitted all initial kernel threads.");
    logln!(
        "LP {}: Bootstrapping complete. Yielding the processor to the scheduler.",
        (get_lp_id())
    );
    YIELD_BARRIER.wait();
    LocalIntCtlr::init_lp();
    yield_lp();
    loop {
        panic!("BSP: Reached end of BSP main function. This should never happen.");
    }
}
/// This is the application processors' entry point into the kernel. The `ap_main` function is
/// called by each application processor upon entering the kernel. It initializes the processor and
/// then hands it off to the scheduler. It is made C ABI compatible so that it can work with the
/// Limine Boot Protocol MP feature. Other boot protocols may require alternate implementations of
/// `ap_main`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ap_main(_cpuinfo: &MpInfo) -> ! {
    unsafe {
        assign_id();
    }
    init::ap_init();
    INIT_BARRIER.wait();
    let lp_id = get_lp_id();
    logln!("LP {lp_id}: Bootstrapping complete.");
    YIELD_BARRIER.wait();
    logln!("LP {lp_id}: Starting local interrupt controller initialization.");
    LocalIntCtlr::init_lp();
    logln!(
        "LP {lp_id}: Initialized local interrupt controller. Yielding the processor to the \
         scheduler."
    );
    yield_lp();
    loop {
        panic!("LP {lp_id}: Reached end of AP main function. This should never happen.");
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn test_fn() {
    loop {}
}
