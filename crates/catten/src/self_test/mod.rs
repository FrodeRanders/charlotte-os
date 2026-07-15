//! # Kernel Self-Test Subsystem
//!
//! This subsystem contains diagnostic tests meant to test the kernel itself and aid in development
//! and troubleshooting. Almost all subsystems with the exception of drivers should have at least
//! some tests in this module. In software engineering terminology the tests in this module should
//! be whitebox integration tests that can be run after Catten initializes itself.

pub mod completion;
pub mod cq;
pub mod cq_completion;
pub mod cq_wait;
pub mod el0;
pub mod el0_demo;
pub mod el0_ipc;
pub mod el0_pingpong;
pub mod el0_service;
pub mod el0_sitas;
pub mod ipc;
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
    memory::object::test_memory_objects();
    completion::test_completion_caps();
    completion::test_detached_operations();
    ipc::test_endpoint_ipc();
    ipc::test_endpoint_ipc_connection_attach();
    ipc::test_endpoint_ipc_connection_copy();
    syscall::test_syscall_dispatch();
    ipi::test_ipi_bounded_queue();
    shard::test_shard_local();
    shard::test_shard_mailbox();
    el0::test_el0_syscall_round_trip();
    el0_ipc::test_el0_endpoint_ipc();
    el0_ipc::test_el0_endpoint_ipc_blocking_receive();
    el0_ipc::test_el0_endpoint_ipc_cross_address_space();
    el0_ipc::test_el0_endpoint_ipc_memory_move();
    el0_ipc::test_el0_endpoint_ipc_memory_copy();
    el0_ipc::test_el0_endpoint_ipc_memory_cancel();
    el0_demo::test_el0_cross_lp_async();
    el0_pingpong::test_el0_ping_pong();
    el0_sitas::test_el0_sitas();
    el0_service::test_el0_service();
    cq::test_cq_ring();
    cq_completion::test_cq_ring_in_completion();
    cq_wait::test_cq_wait_wake();
    logln!("Testing Complete. All Tests Passed!");
}
