#![no_std]
#![no_main]
#![feature(abi_custom)]
#![feature(allocator_api)]
#![feature(extend_one)]
#![feature(iter_advance_by)]
#![feature(likely_unlikely)]
#![feature(slice_ptr_get)]
#![feature(step_trait)]
#![feature(unsafe_cell_access)]
#![allow(static_mut_refs)]
#![allow(named_asm_labels)]

//! # Catten
//!
//! Catten is an operating system kernel developed as a component of CharlotteOS, an
//! experimental modern operating system.This kernel is responsible for initializing the hardware,
//! providing commonizing abstractions for all hardware resources, and managing the execution of
//! user-space applications and the environment in which they run. It is a crucial part of the
//! operating system, as it provides the foundation on which the rest of the system is built and it
//! touches every hardware and software component of the system on which it is used. While it is
//! developed as a component of CharlotteOS, it is designed to be modular and flexible, and thus
//! useful in other operating systems, embedded firmware, and other types of software distributions
//! as well.

extern crate alloc;

pub mod cabi;
pub mod common;
pub mod cpu;
pub mod deferred;
pub mod drivers;
pub mod environment;
pub mod event;
pub mod framebuffer;
pub mod init;
pub mod log;
pub mod memory;
pub mod panic;
pub mod self_test;

use alloc::boxed::Box;

use limine::mp::Cpu;
use spin::{Barrier, Lazy, Mutex};

use crate::cpu::isa::interface::system_info::CpuInfoIfce;
use crate::cpu::isa::lp::ops::get_lp_id;
use crate::cpu::isa::system_info::CpuInfo;
use crate::cpu::isa::timers::print_timer_info;
use crate::cpu::multiprocessor::get_lp_count;
use crate::cpu::multiprocessor::startup::{assign_id, start_secondary_lps};
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::cpu::scheduler::threads::{MASTER_THREAD_TABLE, Thread, ThreadId};
use crate::memory::{KERNEL_ASID, VAddr};

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
        let thread = Thread::new(false, KERNEL_ASID, VAddr::from(test_fn as *const () as usize));
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
    yield_lp!();
}
/// This is the application processors' entry point into the kernel. The `ap_main` function is
/// called by each application processor upon entering the kernel. It initializes the processor and
/// then hands it off to the scheduler. It is made C ABI compatible so that it can work with the
/// Limine Boot Protocol MP feature. Other boot protocols may require alternate implementations of
/// `ap_main`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ap_main(_cpuinfo: &Cpu) -> ! {
    unsafe {
        assign_id();
    }
    init::ap_init();
    INIT_BARRIER.wait();
    logln!(
        "LP {}: Bootstrapping complete. Yielding the processor to the scheduler.",
        (get_lp_id())
    );
    YIELD_BARRIER.wait();
    yield_lp!();
}

#[unsafe(no_mangle)]
pub extern "C" fn test_fn() -> ! {
    let lp_id = get_lp_id();
    mask_interrupts!();
    let tid = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid().unwrap();
    unmask_interrupts!();
    loop {
        mask_interrupts!();
        logln!("LP{lp_id}::T{tid}: Logging from initial thread context.");
        unmask_interrupts!();
    }
}
