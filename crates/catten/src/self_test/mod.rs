//! # Kernel Self-Test Subsystem
//!
//! This subsystem contains diagnostic tests meant to test the kernel itself and aid in development
//! and troubleshooting. Almost all subsystems with the exception of drivers should have at least
//! some tests in this module. In software engineering terminology the tests in this module should
//! be whitebox integration tests that can be run after Catten initializes itself.

pub mod adversarial;
pub mod completion;
pub mod cq;
pub mod cq_completion;
pub mod cq_wait;
pub mod device;
pub mod el0;
pub mod el0_demo;
pub mod el0_ipc;
pub mod el0_net;
pub mod el0_pingpong;
pub mod el0_raft;
pub mod el0_service;
pub mod el0_sitas;
pub mod el0_uart;
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
    ipc::test_vector_ipc_transaction_rollback();
    adversarial::test_adversarial_ipc();
    syscall::test_syscall_dispatch();
    ipi::test_ipi_bounded_queue();
    shard::test_shard_local();
    shard::test_shard_mailbox();
    el0::test_el0_syscall_round_trip();
    el0_raft::test_el0_raft();
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
    device::test_device_capabilities();
    #[cfg(all(feature = "virtio_net_test", not(feature = "hvf_compat"), target_arch = "aarch64"))]
    el0_net::test_el0_net();
    #[cfg(all(feature = "virtio_net_test", feature = "hvf_compat", target_arch = "aarch64"))]
    logln!("Skipping EL0 net test (hvf_compat: HVF cannot emulate EL0 MMIO).");
    #[cfg(all(not(feature = "virtio_net_test"), target_arch = "aarch64"))]
    logln!("Skipping EL0 net test (enable virtio_net_test with matching PCI hardware).");
    el0_uart::test_el0_uart();
    crate::debug_trace::dump_after(10_000);
    logln!("Synchronous self-tests passed; deferred scheduler/EL0 verifiers are still pending.");
}
