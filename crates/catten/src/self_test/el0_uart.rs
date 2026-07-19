//! Self-test: Phase 8 userspace UART driver.
//!
//! Spawns three isolated EL0 protection domains from Rust-compiled ELFs:
//!
//! - `ns.elf` — the userspace name service (registry endpoint delivered by the supervisor through
//!   the bootstrap slot);
//! - `uart.elf` — the reference userspace UART driver, granted a PL011 MMIO register window and the
//!   PL011 interrupt as *capabilities* (plus a bootstrap connection to the name service);
//! - `cclient.elf` — a console client that looks up "uart" by name and writes a short message
//!   through the driver.
//!
//! The driver never names a physical address or an interrupt vector: the
//! supervisor mints the MMIO-region and interrupt capabilities kernel-side
//! and delivers them through the config-page contract. The driver maps the
//! register window into its own address space as EL0 device memory, binds
//! both endpoint readiness and the interrupt to one completion queue, and
//! transmits bytes with direct EL0 MMIO writes.
//!
//! A kernel verifier thread confirms the client completed its writes, then
//! exercises the driver's *delegated* interrupt authority end to end: it
//! software-pends the real PL011 SPI through the GIC and observes the driver
//! acknowledge it from EL0 (success criterion 8).

use crate::logln;
#[cfg(target_arch = "aarch64")]
use crate::{
    ipc::{
        self,
        ConnectionRights,
        IpcError,
    },
    memory::physical::PAddr,
    service::supervisor::{
        self,
        DriverGrant,
        NameServiceHandle,
        ServiceDomain,
    },
};

#[cfg(target_arch = "aarch64")]
const NS_ELF: &[u8] = include_bytes!("ns.elf");
#[cfg(target_arch = "aarch64")]
const UART_ELF: &[u8] = include_bytes!("uart.elf");
#[cfg(target_arch = "aarch64")]
const CCLIENT_ELF: &[u8] = include_bytes!("cclient.elf");

#[cfg(target_arch = "aarch64")]
const fn packed_name(bytes: &[u8]) -> u64 {
    let mut packed = [0u8; 8];
    let mut i = 0;
    while i < bytes.len() && i < 8 {
        packed[i] = bytes[i];
        i += 1;
    }
    u64::from_le_bytes(packed)
}

#[cfg(target_arch = "aarch64")]
const NS_INTERFACE: u64 = packed_name(b"NAME");

/// QEMU `virt` PL011 UART: MMIO base and its GIC SPI (INTID 33 = SPI 1).
#[cfg(target_arch = "aarch64")]
const PL011_BASE: usize = 0x0900_0000;
#[cfg(target_arch = "aarch64")]
const PL011_INTID: u32 = 33;

#[cfg(target_arch = "aarch64")]
const CLIENT_SENTINEL: u32 = 0xc0de;
#[cfg(target_arch = "aarch64")]
const MAX_SPINS: u64 = 80_000_000;

#[cfg(target_arch = "aarch64")]
static mut TEST_STATE: Option<TestState> = None;

#[cfg(target_arch = "aarch64")]
struct TestState {
    name_service: NameServiceHandle,
    driver: Option<ServiceDomain>,
    driver_config: PAddr,
    client_config: PAddr,
}

/// The kernel verifier acts as a console client through the direct kernel
/// API under this pseudo address-space id (it exists only in the IPC
/// capability registry).
#[cfg(target_arch = "aarch64")]
const KCLIENT_ASID: usize = 0x7200;

/// Console protocol opcodes mirrored from `catten-services`.
#[cfg(target_arch = "aarch64")]
const NAME_UART: u64 = packed_name(b"uart");
#[cfg(target_arch = "aarch64")]
const OP_LOOKUP: u32 = 2;
#[cfg(target_arch = "aarch64")]
const OP_WRITE: u32 = 1;
#[cfg(target_arch = "aarch64")]
const OP_READ_DEFERRED: u32 = 4;
#[cfg(target_arch = "aarch64")]
const OP_CRASH: u32 = 5;

pub fn test_el0_uart() {
    #[cfg(target_arch = "aarch64")]
    {
        logln!("Testing EL0 userspace UART driver (delegated MMIO + interrupt)...");

        let name_service = supervisor::spawn_name_service(NS_ELF, NS_INTERFACE, 1, 8);
        let ns_asid = name_service.domain.asid;
        logln!("[uart] name service spawned (asid={})", ns_asid);

        let driver = supervisor::spawn_driver_with_name_service(
            UART_ELF,
            &name_service,
            ConnectionRights::CALL,
            DriverGrant {
                mmio_phys_base: PL011_BASE,
                mmio_pages: 1,
                intid: PL011_INTID,
            },
        );
        let driver_config = driver.config_frame;
        let driver_asid = driver.asid;
        logln!("[uart] driver spawned (asid={}) with PL011 MMIO + IRQ grants", driver_asid);

        let client =
            supervisor::spawn_with_name_service(CCLIENT_ELF, &name_service, ConnectionRights::CALL);
        let client_config = client.config_frame;
        let client_asid = client.asid;
        logln!("[uart] console client spawned (asid={})", client_asid);

        // The client is a fire-and-forget domain observed only through its
        // config page; keep just the frame pointer alive.
        let _ = client;

        unsafe {
            TEST_STATE = Some(TestState {
                name_service,
                driver: Some(driver),
                driver_config,
                client_config,
            });
        }

        let _vtid =
            crate::cpu::scheduler::spawn_thread(crate::memory::KERNEL_ASID, verify_el0_uart);
        logln!("[uart] verifier deferred");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 UART driver test (AArch64 only).");
    }
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_uart() {
    use crate::cpu::scheduler::yield_lp;

    let state = unsafe { TEST_STATE.as_ref() }.expect("[uart] test state missing");

    let client_cfg: *const u32 = {
        let base: *mut u8 = state.client_config.into();
        base as *const u32
    };
    let driver_cfg: *const u32 = {
        let base: *mut u8 = state.driver_config.into();
        base as *const u32
    };

    // --- Phase A: the console client completes lookup → writes → issues a
    // deferred read (stage 5) that only a device interrupt can complete. ---
    {
        let mut spins: u64 = 0;
        while unsafe { core::ptr::read_volatile(client_cfg.add(3)) } < 5 {
            spins += 1;
            if spins % 2_000_000 == 0 {
                let driver_stage = unsafe { core::ptr::read_volatile(driver_cfg) };
                let served = unsafe { core::ptr::read_volatile(driver_cfg.add(3)) };
                let client_stage = unsafe { core::ptr::read_volatile(client_cfg.add(3)) };
                logln!(
                    "[uart] waiting: driver stage {} served {}, client stage {}",
                    driver_stage,
                    served,
                    client_stage
                );
            }
            assert!(spins < MAX_SPINS, "[uart] FAILED waiting for console client writes");
            yield_lp();
        }
    }
    let write_status = unsafe { core::ptr::read_volatile(client_cfg.add(1)) };
    assert_eq!(write_status, 0, "[uart] console write must reply 0");
    let driver_served = unsafe { core::ptr::read_volatile(driver_cfg.add(3)) };
    assert!(driver_served >= 1, "[uart] driver must have served console writes");
    logln!(
        "[uart] console client wrote through the EL0 driver ({} bytes served via PL011 MMIO)",
        driver_served
    );

    // --- Phase B: drive the delegated interrupt. Wait until the driver has
    // actually retained the deferred read (its READ_ARMED marker), then
    // software-pend the real PL011 SPI through the GIC exactly once. The wake
    // is coalesced and not lost even if the driver has not yet re-entered its
    // wait, so a single pend suffices; a rare re-pend guards against a dropped
    // delivery without storming the scheduler with wakes. ---
    {
        let mut spins: u64 = 0;
        while unsafe { core::ptr::read_volatile(driver_cfg.add(1)) } != 1 {
            spins += 1;
            assert!(spins < MAX_SPINS, "[uart] FAILED waiting for driver to arm the deferred read");
            yield_lp();
        }
    }
    crate::cpu::isa::interrupts::gic::set_spi_pending(PL011_INTID);
    {
        let mut spins: u64 = 0;
        while unsafe { core::ptr::read_volatile(client_cfg) } != CLIENT_SENTINEL {
            spins += 1;
            assert!(spins < MAX_SPINS, "[uart] FAILED waiting for interrupt-driven deferred read");
            // Rare safety-net re-pend if the first delivery did not land.
            if spins % 20_000_000 == 0 {
                crate::cpu::isa::interrupts::gic::set_spi_pending(PL011_INTID);
            }
            yield_lp();
        }
    }

    let irq_count = unsafe { core::ptr::read_volatile(driver_cfg.add(2)) };
    assert!(irq_count >= 1, "[uart] driver must have acknowledged a device interrupt");
    // The deferred read result encodes the driver's interrupt count in bits
    // 8.., so a nonzero high half proves the reply was interrupt-driven.
    let read_result = unsafe { core::ptr::read_volatile(client_cfg.add(10)) };
    assert!(
        read_result >> 8 >= 1,
        "[uart] deferred read must have been completed by a device interrupt (got {:#x})",
        read_result
    );

    logln!(
        "[uart] deferred read completed by a delegated device interrupt (result {:#x})",
        read_result
    );

    // --- Phase C: driver restart with device reset and outstanding-operation
    // reconciliation (success criterion 9's driver half). The verifier acts as
    // a second console client through the direct kernel API, leaves a deferred
    // read outstanding, crashes the driver (uncooperative exit that releases
    // nothing), and verifies that teardown reclaims the device authority,
    // reconciles the outstanding call, and that a restarted instance serves a
    // fresh generation. ---
    let state = unsafe { TEST_STATE.as_mut() }.expect("[uart] test state missing");
    let ns_asid = state.name_service.domain.asid;
    let ns_endpoint = state.name_service.endpoint_cap;
    let driver_asid = state.driver.as_ref().expect("[uart] driver domain handle missing").asid;

    let kclient_conn =
        ipc::connection_delegate(ns_asid, ns_endpoint, KCLIENT_ASID, ConnectionRights::CALL)
            .expect("[uart] verifier bootstrap connection failed");
    let lookup = ipc::scalar_call(KCLIENT_ASID, kclient_conn, OP_LOOKUP, NAME_UART)
        .expect("[uart] verifier lookup call failed");
    let reply = wait_reply(lookup, "generation-1 uart lookup reply");
    assert_eq!(reply.result, 1, "[uart] first uart instance must be generation 1");
    let stale_conn = reply.cap.expect("[uart] lookup should return console connection");
    logln!("[uart] verifier got generation-1 console connection");

    // Leave a deferred read outstanding in the driver.
    let orphan_read = ipc::scalar_call(KCLIENT_ASID, stale_conn, OP_READ_DEFERRED, 0)
        .expect("[uart] verifier deferred-read call failed");
    {
        let mut spins: u64 = 0;
        while unsafe { core::ptr::read_volatile(driver_cfg.add(1)) } != 1 {
            spins += 1;
            assert!(spins < MAX_SPINS, "[uart] FAILED waiting for verifier read to arm");
            yield_lp();
        }
    }
    assert_eq!(
        crate::device::interrupt_route_owner(PL011_INTID),
        Some(driver_asid),
        "[uart] live driver must own the PL011 interrupt route"
    );

    // Crash the driver: uncooperative exit, nothing released.
    ipc::scalar_send(KCLIENT_ASID, stale_conn, OP_CRASH, 0).expect("[uart] crash send failed");
    let driver1 = state.driver.take().expect("[uart] driver domain handle missing");
    supervisor::wait_domain_exit(&driver1, MAX_SPINS);
    logln!("[uart] driver crashed (uncooperative exit) with a deferred read outstanding");
    // Tear down promptly: `domain_exited` is true only once the thread has
    // been reaped (and thus switched away from), and reusable thread ids make
    // any delay here race-prone under this test's heavy thread churn.
    supervisor::teardown_domain(driver1);

    // Device reset: teardown must have reclaimed the interrupt route (and the
    // MMIO mapping with the address space).
    assert_eq!(
        crate::device::interrupt_route_owner(PL011_INTID),
        None,
        "[uart] teardown must unroute the crashed driver's interrupt"
    );
    // Outstanding-operation reconciliation: the retained reply token was
    // reclaimed on teardown, so the orphaned deferred read completes as
    // Cancelled instead of hanging.
    let orphan = wait_reply(orphan_read, "orphaned deferred-read reconciliation");
    assert_eq!(
        orphan.result,
        ipc::REPLY_CANCELLED,
        "[uart] orphaned deferred read must be cancelled on driver teardown"
    );
    // Stale connections to the dead instance fail deterministically.
    assert_eq!(
        ipc::scalar_call(KCLIENT_ASID, stale_conn, OP_WRITE, b'X' as u64),
        Err(IpcError::EndpointClosed),
        "[uart] stale console connection must fail EndpointClosed"
    );
    logln!(
        "[uart] teardown reclaimed device authority, cancelled the outstanding read, and stale \
         connections fail EndpointClosed"
    );

    // Restart: a fresh instance with freshly minted device grants.
    let driver2 = supervisor::spawn_driver_with_name_service(
        UART_ELF,
        &state.name_service,
        ConnectionRights::CALL,
        DriverGrant {
            mmio_phys_base: PL011_BASE,
            mmio_pages: 1,
            intid: PL011_INTID,
        },
    );
    let driver2_asid = driver2.asid;
    logln!("[uart] driver restarted (asid={}) with fresh device grants", driver2_asid);

    // Re-lookup until the restarted instance registers with generation 2.
    #[allow(unused_assignments)]
    let mut fresh_conn = 0u64;
    {
        let mut spins: u64 = 0;
        loop {
            let lookup = ipc::scalar_call(KCLIENT_ASID, kclient_conn, OP_LOOKUP, NAME_UART)
                .expect("[uart] verifier re-lookup call failed");
            let reply = wait_reply(lookup, "post-restart uart lookup reply");
            if reply.result == 2 {
                fresh_conn = reply.cap.expect("[uart] re-lookup should return connection");
                break;
            }
            if let Some(cap) = reply.cap {
                let _ = ipc::close_cap(KCLIENT_ASID, cap);
            }
            spins += 1;
            assert!(spins < MAX_SPINS, "[uart] FAILED waiting for generation-2 registration");
            yield_lp();
        }
    }
    let write = ipc::scalar_call(KCLIENT_ASID, fresh_conn, OP_WRITE, b'2' as u64)
        .expect("[uart] generation-2 write call failed");
    let reply = wait_reply(write, "generation-2 console write reply");
    assert_eq!(reply.result, 0, "[uart] generation-2 console write must succeed");
    assert_eq!(
        crate::device::interrupt_route_owner(PL011_INTID),
        Some(driver2_asid),
        "[uart] restarted driver must own the PL011 interrupt route"
    );

    state.driver = Some(driver2);
    ipc::close_address_space(KCLIENT_ASID);

    logln!(
        "[uart] SUCCESS: userspace driver served console writes and an interrupt-driven deferred \
         read from EL0; after an uncooperative driver exit, teardown reclaimed the delegated MMIO \
         and interrupt, cancelled the outstanding deferred read, invalidated stale connections, \
         and a restarted generation-2 instance serves with fresh device grants."
    );
    loop {
        yield_lp();
    }
}

/// Poll a pending call created through the direct kernel API until it
/// completes, then close the pending-call cap.
#[cfg(target_arch = "aarch64")]
fn wait_reply(call: u64, what: &str) -> ipc::ReplyValue {
    use crate::cpu::scheduler::yield_lp;

    #[allow(unused_assignments)]
    let mut value = None;
    let mut spins: u64 = 0;
    loop {
        match ipc::poll_reply(KCLIENT_ASID, call) {
            Ok(Some(reply)) => {
                value = Some(reply);
                break;
            }
            Ok(None) => {}
            Err(error) => panic!("[uart] poll_reply failed for {}: {:?}", what, error),
        }
        spins += 1;
        assert!(spins < MAX_SPINS, "[uart] FAILED waiting for {}", what);
        yield_lp();
    }
    ipc::close_cap(KCLIENT_ASID, call).expect("[uart] pending-call close failed");
    value.expect("[uart] reply value missing")
}
