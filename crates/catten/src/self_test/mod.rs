//! # Kernel Self-Test Subsystem
//!
//! Whitebox integration tests that run after Catten has initialized itself,
//! meant to validate the kernel and aid development and troubleshooting.
//! Almost every subsystem except drivers has coverage here. The suite is
//! compiled into the kernel and invoked from `run_self_tests` on the boot
//! path; it is also the canonical smoke test used by `scripts/run-aarch64.sh`
//! to prove a boot image (virt or `--sbsa-ref`) is healthy.
//!
//! ## Execution model
//!
//! The tests split into two phases, because most of the kernel's *async*
//! machinery requires the scheduler to be running while self-tests start on
//! the boot path before the BSP ever yields:
//!
//! - **Synchronous, boot-path tests** run inline from `run_self_tests` and complete before
//!   `finalize_and_start_coordinator` is called. These exercise kernel APIs that return directly:
//!   the physical/virtual memory allocators, memory-object capability tables, the
//!   completion-capability table, the kernel-side IPC endpoint ABI, syscall dispatch, the bounded
//!   IPI queue, shard-local state/mailboxes, and running-statistics math.
//! - **Deferred verifiers** are kernel threads registered via [`results::spawn_verifier`] that run
//!   once the scheduler is active. Each owns one bit in the authoritative [`results`] bitmap and
//!   reports success/failure through [`results::pass`] / [`results::fail`]. These cover everything
//!   that needs a real scheduler, real user address spaces, real EL0 execution, a live GIC, or a
//!   second guest on the network.
//!
//! ## Test matrix (grouped by subsystem)
//!
//! Memory and heap:
//! - [`memory::pmem`] / [`memory::vmem`] / [`memory::allocator`] — physical and virtual page
//!   allocators and the HHDM/linear mapping; outcome: frames, regions and HHDM aliases are
//!   allocated/freed with correct addresses and no overlap.
//! - [`memory::object`] — memory-object capability tables (map/unmap, borrow, reclaim); outcome:
//!   capabilities round-trip and teardown reclaims pins.
//! - [`statistics`] — running-mean/variance accumulator; outcome: canonical sample set yields the
//!   documented count/min/max/variance.
//!
//! Completion / async syscall ABI:
//! - [`completion`] — completion-capability submission table, buffer-ownership contract, observer
//!   signal path, submission backpressure; outcome: the kernel-side submit/observe path is correct
//!   (the EL0 entry is a later phase).
//! - [`cq`] / [`cq_completion`] — the shared-memory CQ ring producer logic and its integration with
//!   completion; outcome: ring entries appear with the correct values and overflow/pending counts
//!   are exact.
//! - [`cq_wait`] — the blocking, wake-aware CQ wait used by the reactor; outcome: a blocked thread
//!   is released by a completion, by an explicit wake, by a per-shard wake, and by an IPC on a
//!   CQ-bound endpoint.
//!
//! IPC:
//! - [`ipc`] — the kernel-side endpoint IPC ABI (create, mint, delegate, send, call, reply) and
//!   vector-IPC transaction rollback; outcome: endpoints and connections behave as specified and
//!   failed transactions leave no orphaned state.
//! - [`adversarial`] — negative tests (success criterion 12); outcome: every
//!   capability/memory-transfer misuse returns the documented error instead of corrupting kernel
//!   state.
//! - [`syscall`] — the syscall dispatcher; outcome: every dispatch route handles a synthetic
//!   `TrapFrame` correctly.
//!
//! Scheduling and per-LP primitives:
//! - [`ipi`] — the bounded cross-LP IPI queue; outcome: try-push reports backpressure when full and
//!   closures drain/execute.
//! - [`shard`] — lock-free `ShardLocal<T>` and typed `ShardMailbox<M>`; outcome: owner/borrow
//!   discipline and bounded send/receive hold.
//! - [`scheduler_lifecycle`] — timer, migration and cross-LP retirement; outcome: threads
//!   migrate/retire without lost timers or dangling LP state.
//! - [`statistics`] is exercised here as a kernel utility.
//!
//! EL0 (userspace) execution — each spawns a real protection domain and
//! verifies the SVC ABI + capability model end to end:
//! - [`el0`] — a hand-written stub executes a syscall at EL0; outcome: the syscall round-trips and
//!   writes its result back.
//! - [`el0_ipc`] — endpoint IPC from EL0 (create, mint, delegate, call, receive, reply, blocking
//!   receive, cross-AS, memory-move/copy/cancel); outcome: the scalar endpoint ABI works from
//!   userspace with no ASID/LP leakage.
//! - [`el0_demo`] — async + cross-LP work placement via the syscall ABI; outcome: a worker pinned
//!   to another LP completes a shared-CQ round trip.
//! - [`el0_pingpong`] — two shards communicate cross-LP over the full svc ABI; outcome: the
//!   mailbox/capability handshake completes with correct data.
//! - [`el0_sitas`] — loads the Rust-compiled sitas/catten-user ELF and runs `basic_kv`; outcome: a
//!   real ELF's PT_LOAD segments map and execute.
//! - [`el0_service`] — the name service + service manager (Phase 3); outcome: services
//!   register/lookup by name and restart semantics (teardown, stale-connection rejection,
//!   generation bump) hold.
//! - [`el0_raft`] — two-node Raft leader election and NVMe-backed persistent recovery; outcome:
//!   exactly one leader is elected and a restarted node recovers and advances its durable term.
//!
//! Device model:
//! - [`device`] — device capabilities (MMIO regions, interrupt objects); outcome: grants/unmaps and
//!   interrupt delivery to a completion queue work both via the kernel path and through a real GIC
//!   SPI.
//! - [`el0_uart`] — Phase 8 userspace UART driver; outcome: a client writes through a real EL0
//!   driver and a delegated PL011 interrupt completes a deferred read; teardown/restart reclaim and
//!   re-grant cleanly.
//! - [`el0_nvme`] — Phase 1 NVMe block driver + object store; outcome: a 12 KiB write/flush/read
//!   round trip with real MSI-X completions, an object-store format/mount, and (with storage)
//!   persistent Raft recovery.
//!
//! Networking (feature-gated, `target_arch = "aarch64"`):
//! - [`el0_net`] (`virtio_net_test`), [`el0_disco`] (`disco_net_test`), [`el0_dns`]
//!   (`dns_net_test`), [`el0_tcpip`] (`tcpip_net_test`), [`el0_http`] (`http_net_test`) —
//!   virtio-net, cluster discovery, the distributed name service over Raft, the smoltcp adapter,
//!   and an HTTP server. These need matching PCI hardware (or a second QEMU guest) and are skipped
//!   in the ordinary disk-only build.
//!
//! ## Results and expected outcome
//!
//! [`results`] is the authoritative completion tracker. The boot suite
//! registers 18 tests; each network feature adds one more. A deferred
//! coordinator thread prints periodic `SELFTEST WAITING` summaries and, when
//! every registered test has either passed or failed, a single final line:
//!
//! ```text
//! SELFTEST COMPLETE: passed=18 failed=0 pending=0 passed_bitmap=0x3ffff ...
//! ```
//!
//! **Expected outcome on both virt/TCG and sbsa-ref: `passed=18 failed=0
//! pending=0`** (the bitmap `0x3ffff` covers the 18 registered boot tests).
//! `run_self_tests` itself returns after the synchronous tests; the final
//! authoritative verdict is produced by the coordinator thread and observed
//! by `scripts/run-aarch64.sh` under `--timeout`.

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
