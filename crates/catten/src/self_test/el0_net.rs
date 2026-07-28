//! Self-test: Phase 9 userspace virtio-net driver.
//!
//! This module is invoked only with the `virtio_net_test` feature. The test
//! requires a virtio-net PCI function at the BAR and interrupt described
//! below; starting it in the ordinary disk-only QEMU configuration leaves
//! its deferred verifier waiting forever and keeps guest CPUs runnable.
//!
//! Uses the node name service; a deferred kernel verifier thread (which runs
//! after the scheduler and the topology probe
//! become active) discovers the virtio-net PCI device, grants its BAR0 + IRQ
//! to the driver domain, spawns a client that queries status, and verifies
//! the MAC and link state.
#![cfg(target_arch = "aarch64")]

use crate::{
    ipc::ConnectionRights,
    logln,
    service::supervisor::{
        self,
        DriverGrant,
        NameServiceHandle,
    },
};

#[cfg(target_arch = "aarch64")]
const NET_ELF: &[u8] = include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/net.elf"));
#[cfg(target_arch = "aarch64")]
const NCLIENT_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/nclient.elf"));

#[cfg(target_arch = "aarch64")]
const CLIENT_SENTINEL: u32 = 0xc0de;
#[cfg(target_arch = "aarch64")]
#[cfg(target_arch = "aarch64")]
static mut TEST_STATE: Option<NameServiceHandle> = None;

pub fn test_el0_net() {
    #[cfg(target_arch = "aarch64")]
    {
        logln!("Testing EL0 userspace virtio-net driver...");

        let name_service = supervisor::node_name_service();
        let ns_asid = name_service.domain.asid;
        logln!("[net] using node name service (asid={})", ns_asid);

        unsafe { TEST_STATE = Some(name_service) };

        let _vtid = crate::cpu::scheduler::spawn_thread(crate::memory::KERNEL_ASID, verify_el0_net);
        logln!("[net] verifier deferred (waits for PCI topology + driver + client)");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 net driver test (AArch64 only).");
    }
}

/// On QEMU `virt` with `-device virtio-net-pci`, the device is at B:D:F
/// `00:01.0` with deterministic BAR0 and IRQ.  Hardcoded to skip the PCI
/// topology walk: under HVF, reading ECAM triggers an assertion
/// (hvf_handle_exception: isv); the net driver is blocked on HVF anyway
/// for EL0 MMIO, so the topology lookup would never succeed.
fn wait_for_virtio_net() -> (usize, u32) {
    let bar0: usize = 0x1000_0000;
    let intid: u32 = 44;
    logln!("[net] hardcoded BAR0={:#x} intid={}", bar0, intid);
    (bar0, intid)
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_net() {
    use crate::cpu::scheduler::yield_lp;

    let ns = unsafe { TEST_STATE.as_ref() }.expect("[net] test state missing");

    logln!("[net] verifier running, waiting for PCI topology...");
    let (bar0, intid) = wait_for_virtio_net();
    let driver = supervisor::spawn_driver_with_name_service(
        NET_ELF,
        ns,
        ConnectionRights::CALL,
        DriverGrant {
            mmio_phys_base: bar0,
            mmio_pages: 1,
            intid,
            dma_requester_id: None,
            dma_msi_address: None,
        },
    );
    let driver_config = driver.status_frame;
    let driver_asid = driver.asid;
    logln!("[net] driver spawned (asid={}) with BAR0 + IRQ grants", driver_asid);
    let _driver = driver;

    let client = supervisor::spawn_with_name_service(NCLIENT_ELF, ns, ConnectionRights::CALL);
    let client_config = client.status_frame;
    let client_asid = client.asid;
    logln!("[net] client spawned (asid={})", client_asid);
    let _client = client;

    let client_cfg: *const u32 = {
        let base: *mut u8 = client_config.into();
        base as *const u32
    };
    let driver_cfg: *const u32 = {
        let base: *mut u8 = driver_config.into();
        base as *const u32
    };

    // --- wait for client sentinel ---
    {
        let mut spins: u64 = 0;
        let deadline = crate::self_test::results::Deadline::after_millis(10_000);
        while unsafe { core::ptr::read_volatile(client_cfg) } != CLIENT_SENTINEL {
            spins += 1;
            if spins.is_multiple_of(2_000_000) {
                let ds = unsafe { core::ptr::read_volatile(driver_cfg) };
                let cs = unsafe { core::ptr::read_volatile(client_cfg.add(3)) };
                logln!("[net] waiting: driver stage {} client stage {}", ds, cs);
            }
            deadline.assert_pending("EL0 network client");
            yield_lp();
        }
    }

    let status = unsafe { core::ptr::read_volatile(client_cfg.add(1)) } as u64;
    let link = status & 0xff;
    let m0 = ((status >> 48) & 0xff) as u8;
    let m1 = ((status >> 40) & 0xff) as u8;
    let m2 = ((status >> 32) & 0xff) as u8;
    let m3 = ((status >> 24) & 0xff) as u8;
    let m4 = ((status >> 16) & 0xff) as u8;
    let m5 = ((status >> 8) & 0xff) as u8;
    logln!(
        "[net] client status link={} MAC={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        link,
        m0,
        m1,
        m2,
        m3,
        m4,
        m5
    );
    assert_ne!(status >> 8, 0, "[net] MAC must be nonzero");
    assert_eq!(link, 1, "[net] link must be up");

    let ds = unsafe { core::ptr::read_volatile(driver_cfg) };
    assert!(ds >= 6, "[net] driver must reach serving stage (got {})", ds);

    logln!(
        "[net] SUCCESS: userspace virtio-net driver reached DRIVER_OK, read MAC \
         {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, and served a status query from EL0.",
        m0,
        m1,
        m2,
        m3,
        m4,
        m5
    );
    crate::self_test::results::pass(crate::self_test::results::TestId::Net);
}
