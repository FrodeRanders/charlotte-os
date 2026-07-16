//! Self-test: Phase 9 userspace virtio-net driver.
//!
//! Spawns the name service synchronously during self-tests; a deferred kernel
//! verifier thread (which runs after the scheduler and the topology probe
//! become active) discovers the virtio-net PCI device, grants its BAR0 + IRQ
//! to the driver domain, spawns a client that queries status, and verifies
//! the MAC and link state.
#![cfg(target_arch = "aarch64")]

use crate::{
    ipc::ConnectionRights,
    service::supervisor::{
        self,
        DriverGrant,
        NameServiceHandle,
    },
};
use crate::logln;

#[cfg(target_arch = "aarch64")]
const NS_ELF: &[u8] = include_bytes!("ns.elf");
#[cfg(target_arch = "aarch64")]
const NET_ELF: &[u8] = include_bytes!("net.elf");
#[cfg(target_arch = "aarch64")]
const NCLIENT_ELF: &[u8] = include_bytes!("nclient.elf");

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
#[cfg(target_arch = "aarch64")]
const CLIENT_SENTINEL: u32 = 0xC0DE;
#[cfg(target_arch = "aarch64")]
const MAX_SPINS: u64 = 80_000_000;

#[cfg(target_arch = "aarch64")]
static mut TEST_STATE: Option<NameServiceHandle> = None;

pub fn test_el0_net() {
    #[cfg(target_arch = "aarch64")]
    {
        logln!("Testing EL0 userspace virtio-net driver...");

        let name_service = supervisor::spawn_name_service(NS_ELF, NS_INTERFACE, 1, 8);
        let ns_asid = name_service.domain.asid;
        logln!("[net] name service spawned (asid={})", ns_asid);

        unsafe { TEST_STATE = Some(name_service) };

        let _vtid =
            crate::cpu::scheduler::spawn_thread(crate::memory::KERNEL_ASID, verify_el0_net);
        logln!("[net] verifier deferred (waits for PCI topology + driver + client)");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 net driver test (AArch64 only).");
    }
}

/// Poll the PCI topology until the virtio-net device is discovered, then
/// return its BAR0 physical base and SPI id.
fn wait_for_virtio_net() -> (usize, u32) {
    use crate::cpu::scheduler::yield_lp;
    use crate::device_management::drivers::busses::pci_express::topology;
    use crate::device_management::topology::DEVICE_TOPOLOGY;

    let mut spins: u64 = 0;
    loop {
        if let Some((phys_base, irq_line)) = topology::lookup_first_virtio_net(&DEVICE_TOPOLOGY.pcie)
        {
            let intid = (irq_line as u32) + 32;
            logln!("[net] PCI lookup BAR0={:#x} irq_line={} intid={}", phys_base, irq_line, intid);
            return (phys_base as usize, intid);
        }
        spins += 1;
        if spins % 4_000_000 == 0 {
            logln!("[net] still waiting for virtio-net in PCI topology ({} spins)", spins);
        }
        assert!(spins < MAX_SPINS, "[net] FAILED waiting for virtio-net device in PCI topology");
        yield_lp();
    }
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
        },
    );
    let driver_config = driver.config_frame;
    let driver_asid = driver.asid;
    logln!("[net] driver spawned (asid={}) with BAR0 + IRQ grants", driver_asid);
    let _driver = driver;

    let client =
        supervisor::spawn_with_name_service(NCLIENT_ELF, ns, ConnectionRights::CALL);
    let client_config = client.config_frame;
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
        while unsafe { core::ptr::read_volatile(client_cfg) } != CLIENT_SENTINEL {
            spins += 1;
            if spins % 2_000_000 == 0 {
                let ds = unsafe { core::ptr::read_volatile(driver_cfg) };
                let cs = unsafe { core::ptr::read_volatile(client_cfg.add(3)) };
                logln!("[net] waiting: driver stage {} client stage {}", ds, cs);
            }
            assert!(spins < MAX_SPINS, "[net] FAILED waiting for net client");
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
        link, m0, m1, m2, m3, m4, m5
    );
    assert_ne!(status >> 8, 0, "[net] MAC must be nonzero");
    assert_eq!(link, 1, "[net] link must be up");

    let ds = unsafe { core::ptr::read_volatile(driver_cfg) };
    assert!(ds >= 6, "[net] driver must reach serving stage (got {})", ds);

    logln!("[net] SUCCESS: userspace virtio-net driver reached DRIVER_OK, read MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, and served a status query from EL0.",
        m0, m1, m2, m3, m4, m5);
    loop {
        yield_lp();
    }
}
