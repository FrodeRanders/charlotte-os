//! # Kernel Self-Test Subsystem
//!
//! This subsystem contains diagnostic tests meant to test the kernel itself and aid in development
//! and troubleshooting. Almost all subsystems with the exception of drivers should have at least
//! some tests in this module. In software engineering terminology the tests in this module should
//! be whitebox integration tests that can be run after Catten initializes itself.

pub mod completion;
pub mod cq;
pub mod el0;
pub mod ipi;
pub mod memory;
pub mod shard;
pub mod syscall;

use crate::logln;

pub fn run_self_tests() {
    logln!("Running self tests...");
    // These raw probes target specific x86-64 HHDM/heap virtual addresses used
    // during heap debugging; they are not valid on other architectures.
    #[cfg(target_arch = "x86_64")]
    {
        let probe = 0xffff8400001ffff8usize as *const usize; // heap vaddr -> phys 0x3ffff8
        let hhdm = 0xffff8000003ffff8usize as *const usize; // HHDM alias of phys 0x3ffff8
        crate::early_logln!(
            "[HEAPDBG] probe@start heap={:#x} hhdm={:#x}",
            (unsafe { probe.read() }),
            (unsafe { hhdm.read() })
        );
    }
    memory::pmem::test_pmem();
    memory::vmem::test_vmem();
    memory::allocator::test_allocator();
    completion::test_completion_caps();
    syscall::test_syscall_dispatch();
    ipi::test_ipi_bounded_queue();
    shard::test_shard_local();
    shard::test_shard_mailbox();
    el0::test_el0_syscall_round_trip();
    cq::test_cq_ring();
    logln!("Testing Complete. All Tests Passed!");
}
