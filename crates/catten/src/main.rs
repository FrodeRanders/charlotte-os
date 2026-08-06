#![no_std]
#![no_main]
#![feature(extend_one)]
#![feature(iter_advance_by)]
#![feature(likely_unlikely)]
#![feature(step_trait)]
#![cfg_attr(target_arch = "x86_64", feature(abi_custom))]
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

pub mod capability;
pub mod completion;
pub mod cpu;
pub mod debug_trace;
pub mod deferred_work_manager;
pub mod demo;
pub mod device;
pub mod device_management;
pub mod environment;
pub mod init;
pub mod ipc;
pub mod klib;
pub mod log;
pub mod memory;
pub mod panic;
pub mod self_test;
pub mod service;
pub mod syscall;
pub mod timers;

use core::hint::unreachable_unchecked;

use limine::mp::MpInfo;
use spin::{
    Barrier,
    LazyLock,
};

#[cfg(all(not(feature = "hvf_compat"), not(feature = "live_upgrade_test")))]
use crate::{
    cpu::scheduler::spawn_thread,
    memory::KERNEL_ASID,
};
use crate::{
    cpu::{
        isa::{
            interface::{
                interrupts::LocalIntCtlrIfce,
                system_info::CpuInfoIfce,
            },
            interrupts::LocalIntCtlr,
            lp::ops::get_lp_id,
            system_info::CpuInfo,
            timers::print_timer_info,
        },
        multiprocessor::{
            get_lp_count,
            startup::{
                assign_id,
                start_secondary_lps,
            },
        },
        scheduler::{
            system_scheduler::SYSTEM_SCHEDULER,
            yield_lp,
        },
    },
    device_management::topology::DEVICE_TOPOLOGY,
};

const KERNEL_VERSION: (u64, u64, u64) = (0, 8, 1);
static INIT_BARRIER: LazyLock<Barrier> = LazyLock::new(|| Barrier::new(get_lp_count() as usize));
static INTERRUPT_INIT_BARRIER: LazyLock<Barrier> =
    LazyLock::new(|| Barrier::new(get_lp_count() as usize));
static YIELD_BARRIER: LazyLock<Barrier> = LazyLock::new(|| Barrier::new(get_lp_count() as usize));
/// The kernel entry point, linked as the ELF entry (`ENTRY(_start)` in
/// `linker/aarch64.ld`).
///
/// Limine base revision 6 enters a kernel at **EL2 with VHE** when the boot
/// firmware hands off at EL2 (e.g. QEMU `sbsa-ref`, ARM servers). This kernel
/// targets EL1/EL0, so the entry descends to EL1 before `bsp_main`: the MMU and
/// interrupt setup all program EL1 system registers, and EL2 (an optional
/// hypervisor level) is not where the OS wants to live.
///
/// On EL2 entry, Limine has already built the kernel's page tables and left
/// them in the VHE-redirected EL2 register bank (`TTBR0_EL1`/`TTBR1_EL1`/...).
/// We capture that state, clear `HCR_EL2.E2H` so the `*_EL1` names address the
/// real EL1 bank, program EL1's MMU, and `eret` down to EL1h. On EL1 entry
/// (virt) we simply continue.
///
/// # Safety
/// This is the raw EL1/EL2 entry point invoked by the bootloader (Limine):
/// it must be entered with the expected exception level and a valid kernel
/// image, and it does not return.
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        // The EL field is bits 3:2; EL2 is 0b10 (0x8).
        "mrs x1, CurrentEL",
        "and x1, x1, #0xc",
        "cmp x1, #0x8",
        "b.ne 2f",
        // --- At EL2 (VHE per Limine's handoff) ---
        // Capture the kernel's MMU state from the VHE-redirected EL2 bank.
        "mrs x2, ttbr0_el1",
        "mrs x3, ttbr1_el1",
        "mrs x4, tcr_el1",
        "mrs x5, mair_el1",
        "mrs x6, sctlr_el1",
        // Clear E2H so the *_EL1 names now address the EL1 register bank.
        "mrs x7, hcr_el2",
        "bic x7, x7, #(1 << 34)",
        "msr hcr_el2, x7",
        "isb",
        // Program the EL1 bank with the kernel's page tables.
        "msr ttbr0_el1, x2",
        "msr ttbr1_el1, x3",
        "msr tcr_el1, x4",
        "msr mair_el1, x5",
        "msr sctlr_el1, x6",
        "isb",
        "dsb sy",
        "isb",
        // Descend to EL1h at the continuation below.
        "adrp x0, 2f",
        "add x0, x0, #:lo12:2f",
        "msr elr_el2, x0",
        "mov x0, #0x5", // SPSR.M[3:0] = EL1h
        "msr spsr_el2, x0",
        "isb",
        "eret",
        "2:",
        // Now at EL1 with the kernel's own page tables active.
        "b {entry}",
        entry = sym bsp_main,
    );
}

/// This is the bootstrap processor's entry point into the kernel. The `bsp_main` function is
/// called by the bootloader after setting up the environment. It is made C ABI compatible so
/// that it can be called by Limine or any other Limine Boot Protocol compliant bootloader.
#[unsafe(no_mangle)]
pub extern "C" fn bsp_main() -> ! {
    crate::log::init_timestamp_epoch();
    #[cfg(target_arch = "aarch64")]
    {
        crate::cpu::isa::lp::ops::enable_fp_simd();
        crate::log::serial::init();
    }
    #[cfg(target_arch = "x86_64")]
    {
        crate::log::serial_x86::init();
    }
    early_logln!(
        "Catten Kernel Version {}.{}.{}",
        (KERNEL_VERSION.0),
        (KERNEL_VERSION.1),
        (KERNEL_VERSION.2)
    );
    early_logln!("========================================================================");
    early_logln!("Initializing the system using the bootstrap processor...");
    unsafe {
        assign_id();
    }
    early_logln!("BSP assigned ID 0.");
    init::bsp_init();
    // Construct global queues that can later be reached from interrupt or
    // preemptible runtime context while execution is still BSP-only. Their
    // `spin::LazyLock` once state must never become another spin dependency on
    // a preempted initializer.
    crate::device::prepare_interrupt_ingress();
    spin::LazyLock::force(&crate::cpu::multiprocessor::ipi::IPI_CMD_QUEUES);
    spin::LazyLock::force(&crate::deferred_work_manager::DWM);
    logln!("System initialized.");
    logln!("Starting secondary LPs...");
    start_secondary_lps().expect("Failed to start secondary LPs");
    INIT_BARRIER.wait();
    // Deferred verifiers and EL0 bootstrap services may begin running while
    // `run_self_tests` is still registering the suite. Interrupt delivery must
    // therefore be ready before any of those threads are admitted, rather than
    // being deferred until the final hand-off to the scheduler.
    mask_interrupts!();
    LocalIntCtlr::init_lp();
    INTERRUPT_INIT_BARRIER.wait();
    #[cfg(target_arch = "aarch64")]
    {
        let name_service = crate::service::supervisor::start_node_name_service();
        logln!(
            "[node] name service started (asid={}, tid={})",
            name_service.domain.asid,
            name_service.domain.tid
        );
        let observer = crate::service::supervisor::start_observability_service(&name_service);
        logln!(
            "[node] observability service started (asid={}, tid={})",
            observer.asid,
            observer.tid
        );
    }
    #[cfg(all(
        target_arch = "aarch64",
        not(feature = "hvf_compat"),
        not(feature = "live_upgrade_test")
    ))]
    match crate::device::smmu::initialize_early() {
        Ok(()) => logln!("[smmu] early initialization complete."),
        Err(crate::device::smmu::Error::Unsupported) => {
            logln!("[smmu] no supported SMMUv3 discovered; DMA isolation unavailable.")
        }
        Err(error) => panic!("[smmu] early initialization failed: {:?}", error),
    }
    self_test::run_synchronous_self_tests();
    // The remaining boot work resolves store-backed ELFs and therefore may
    // yield while the NVMe/object-store bootstrap domains run. Execute it in a
    // scheduler-owned context: yielding from this raw BSP entry stack would
    // abandon the continuation because it has no Thread record to re-admit.
    let continuation = crate::cpu::scheduler::spawn_thread_on_lp(
        crate::memory::KERNEL_ASID,
        finish_boot,
        get_lp_id(),
    );
    logln!("Boot continuation spawned with ID = {continuation}.");
    // The global-state probes above are complete. Release the APs into their
    // idle schedulers before the continuation admits EL0 work; store-backed
    // service discovery may then yield and make progress on any LP.
    YIELD_BARRIER.wait();
    unmask_interrupts!();
    yield_lp();
    /* We've switched into thread context and never come back. */
    unsafe { unreachable_unchecked() }
}

/// Finish boot from a scheduler-owned kernel thread.
///
/// Store-backed service resolution can block cooperatively, so this phase may
/// be suspended and later resumed just like every other kernel thread.
extern "C" fn finish_boot() {
    // The synchronous half of the self-test suite predates kernel preemption
    // and deliberately runs as one transaction. Keep this continuation
    // locally non-preemptible; explicit yields still switch to EL0/services,
    // whose own saved PSTATE enables interrupt delivery while they run.
    mask_interrupts!();
    self_test::run_deferred_self_tests();
    logln!("System Information:");
    logln!("CPU Vendor: {}", (CpuInfo::get_vendor()));
    logln!("CPU Model: {}", (CpuInfo::get_model()));
    logln!("Physical Address bits implemented: {}", (CpuInfo::get_paddr_sig_bits()));
    logln!("Virtual Address bits implemented: {}", (CpuInfo::get_vaddr_sig_bits()));
    print_timer_info();
    #[cfg(feature = "acpi")]
    {
        environment::acpi::print_table_map();
    }
    mask_interrupts!();
    #[cfg(all(not(feature = "hvf_compat"), not(feature = "live_upgrade_test")))]
    {
        // Construct and publish the immutable topology while boot is still
        // single-threaded. Deferred driver verifiers run as soon as the
        // scheduler starts; allowing one of them to race this LazyLock's first
        // initialization made device discovery depend on scheduling order.
        spin::LazyLock::force(&DEVICE_TOPOLOGY);
        logln!("Spawning initial kernel thread to probe device topology...");
        let thread_id = spawn_thread(KERNEL_ASID, probe_device_topology);
        logln!("Initial thread spawned with ID = {thread_id}.");
    }
    #[cfg(all(feature = "hvf_compat", not(feature = "live_upgrade_test")))]
    logln!("PCI topology probe skipped (hvf_compat: ECAM MMIO triggers HVF assertion).");
    // Spawn the async-syscall demonstration (submit -> async worker -> complete
    // -> wake), exercising the completion ABI end-to-end once the scheduler is
    // active.
    #[cfg(not(feature = "live_upgrade_test"))]
    crate::demo::spawn_async_syscall_demo();
    // Admit the controlled scheduler-rebalancing workload last so its initial
    // co-location cannot be cancelled out by later least-loaded admissions.
    #[cfg(not(feature = "live_upgrade_test"))]
    crate::self_test::scheduler_lifecycle::test_scheduler_lifecycle();
    crate::self_test::results::finalize_and_start_coordinator();
    // Publish the node's boot-done marker once the boot storm settles, so
    // network-initiating services wait until this node is through boot before
    // communicating with the rest of the cluster.
    #[cfg(target_arch = "aarch64")]
    crate::service::supervisor::start_boot_done_publisher();
    // Initial admission is intentionally affinity-preserving. Once the full
    // boot workload is known, migrate explicitly certified Ready work from
    // overloaded LPs before any of those contexts begin executing.
    #[cfg(not(feature = "live_upgrade_test"))]
    {
        let mut rebalanced = 0usize;
        while crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER.read().try_rebalance() {
            rebalanced += 1;
        }
        logln!(
            "[scheduler rebalance] moved {} certified Ready thread(s) at boot quiescence.",
            rebalanced
        );
    }
    logln!("Submitted all initial kernel threads.");
    unmask_interrupts!();
}

/// This is the application processors' entry point into the kernel. The `ap_main` function is
/// called by each application processor upon entering the kernel. It initializes the processor and
/// then hands it off to the scheduler. It is made C ABI compatible so that it can work with the
/// Limine Boot Protocol MP feature. Other boot protocols may require alternate implementations of
/// `ap_main`.
///
/// # Safety
///
/// The bootloader must invoke this exactly once per application processor
/// with valid Limine MP state after BSP-owned global initialization.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ap_main(_cpuinfo: &MpInfo) -> ! {
    #[cfg(target_arch = "aarch64")]
    crate::cpu::isa::lp::ops::enable_fp_simd();
    unsafe {
        assign_id();
    }
    init::ap_init();
    mask_interrupts!();
    INIT_BARRIER.wait();
    let lp_id = get_lp_id();
    logln!("LP {lp_id}: Starting local interrupt controller initialization.");
    LocalIntCtlr::init_lp();
    logln!("LP {lp_id}: Initialized local interrupt controller.");
    INTERRUPT_INIT_BARRIER.wait();
    logln!("LP {lp_id}: Bootstrapping complete.");
    YIELD_BARRIER.wait();
    unmask_interrupts!();
    yield_lp();
    /* We've switched into thread context and never come back */
    unsafe { unreachable_unchecked() }
}

#[unsafe(no_mangle)]
pub extern "C" fn probe_device_topology() {
    logln!("LP {}: Probing device topology...", (get_lp_id()));
    let device_topology = &*DEVICE_TOPOLOGY;
    logln!("LP {}: Device Topology:\n{}", (get_lp_id()), device_topology);
}

#[unsafe(no_mangle)]
pub extern "C" fn test_fn() {
    let thread_id = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid().unwrap();
    let lp_id = get_lp_id();
    loop {
        logln!("Logging from thread {thread_id} on LP {lp_id}!");
    }
}
