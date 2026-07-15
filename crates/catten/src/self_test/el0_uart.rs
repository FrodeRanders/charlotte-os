//! Self-test: Phase 8 userspace UART driver.
//!
//! Spawns three isolated EL0 protection domains from Rust-compiled ELFs:
//!
//! - `ns.elf` — the userspace name service (registry endpoint delivered by
//!   the supervisor through the bootstrap slot);
//! - `uart.elf` — the reference userspace UART driver, granted a PL011 MMIO
//!   register window and the PL011 interrupt as *capabilities* (plus a
//!   bootstrap connection to the name service);
//! - `cclient.elf` — a console client that looks up "uart" by name and writes
//!   a short message through the driver.
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

#[cfg(target_arch = "aarch64")]
use crate::{
    ipc::ConnectionRights,
    memory::physical::PAddr,
    service::supervisor::{
        self,
        DriverGrant,
        NameServiceHandle,
        ServiceDomain,
    },
};
use crate::logln;

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
const CLIENT_SENTINEL: u32 = 0xC0DE;
#[cfg(target_arch = "aarch64")]
const MAX_SPINS: u64 = 80_000_000;

#[cfg(target_arch = "aarch64")]
static mut TEST_STATE: Option<TestState> = None;

#[cfg(target_arch = "aarch64")]
struct TestState {
    #[allow(dead_code)]
    name_service: NameServiceHandle,
    #[allow(dead_code)]
    driver: ServiceDomain,
    driver_config: PAddr,
    client_config: PAddr,
}

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
                driver,
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
        "[uart] SUCCESS: userspace driver mapped delegated MMIO, served console writes through \
         PL011 registers from EL0, and completed a deferred read driven by a delegated device \
         interrupt (result {:#x}).",
        read_result
    );
    loop {
        yield_lp();
    }
}
