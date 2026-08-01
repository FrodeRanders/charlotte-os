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
#[cfg(all(feature = "disco_net_test", target_arch = "aarch64"))]
pub mod el0_disco;
#[cfg(all(feature = "dns_net_test", target_arch = "aarch64"))]
pub mod el0_dns;
#[cfg(all(feature = "http_net_test", target_arch = "aarch64"))]
pub mod el0_http;
pub mod el0_ipc;
#[cfg(target_arch = "aarch64")]
pub mod el0_net;
#[cfg(target_arch = "aarch64")]
pub mod el0_nvme;
pub mod el0_pingpong;
pub mod el0_raft;
#[cfg(target_arch = "aarch64")]
pub mod el0_service;
pub mod el0_sitas;
#[cfg(all(feature = "tcpip_net_test", target_arch = "aarch64"))]
pub mod el0_tcpip;
#[cfg(target_arch = "aarch64")]
pub mod el0_uart;
pub mod ipc;
pub mod ipi;
pub mod memory;
pub mod results;
pub mod scheduler_lifecycle;
pub mod shard;
pub mod statistics;
pub mod syscall;

use crate::logln;

/// Status-frame address of the frame demultiplexer (frouter), published by
/// the net test and read by the disco verifier for diagnostics. Zero until
/// the frouter has been spawned.
pub static mut FROUTER_STATUS_FRAME: usize = 0;

/// Synchronous self-tests that construct address spaces directly sometimes
/// retain only their numeric id. Confine that downgrade here; production
/// lifecycle code has no raw-ASID teardown API and must retain an
/// `AddressSpaceHandle` from allocation.
pub(crate) fn close_test_address_space(
    asid: crate::memory::AddressSpaceId,
) -> Result<(), crate::memory::AddressSpaceCloseError> {
    let handle = crate::memory::current_address_space_handle(asid)
        .ok_or(crate::memory::AddressSpaceCloseError::AddressSpaceMissing)?;
    crate::memory::close_user_address_space_handle(handle)
}

pub fn run_self_tests() {
    logln!("Running self tests...");
    if cfg!(feature = "live_upgrade_test") {
        #[cfg(not(target_arch = "aarch64"))]
        panic!("live_upgrade_test requires AArch64 EL0 service images");
        #[cfg(target_arch = "aarch64")]
        {
            results::register(results::TestId::Service);
            el0_service::test_el0_service();
            logln!("Live-upgrade verifier is pending.");
            return;
        }
    }
    results::register_boot_suite();
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
    statistics::test_running_statistics();
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
    #[cfg(target_arch = "aarch64")]
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
    #[cfg(all(feature = "disco_net_test", target_arch = "aarch64"))]
    el0_disco::test_el0_disco();
    #[cfg(all(not(feature = "disco_net_test"), target_arch = "aarch64"))]
    logln!("Skipping EL0 disco test (enable disco_net_test with matching PCI hardware).");
    #[cfg(all(feature = "dns_net_test", target_arch = "aarch64"))]
    el0_dns::test_el0_dns();
    #[cfg(all(not(feature = "dns_net_test"), target_arch = "aarch64"))]
    logln!("Skipping EL0 dns test (enable dns_net_test with matching PCI hardware).");
    #[cfg(all(feature = "tcpip_net_test", target_arch = "aarch64"))]
    el0_tcpip::test_el0_tcpip();
    #[cfg(all(not(feature = "tcpip_net_test"), target_arch = "aarch64"))]
    logln!("Skipping EL0 tcpip test (enable tcpip_net_test with matching PCI hardware).");
    #[cfg(all(feature = "http_net_test", target_arch = "aarch64"))]
    el0_http::test_el0_http();
    #[cfg(all(not(feature = "http_net_test"), target_arch = "aarch64"))]
    logln!("Skipping EL0 http test (enable http_net_test with matching PCI hardware).");
    #[cfg(target_arch = "aarch64")]
    el0_nvme::test_el0_nvme();
    #[cfg(target_arch = "aarch64")]
    el0_uart::test_el0_uart();
    logln!("Synchronous self-tests passed; deferred scheduler/EL0 verifiers are still pending.");
}
