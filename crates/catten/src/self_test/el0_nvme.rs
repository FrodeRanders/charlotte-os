//! Self-test: Phase 1 userspace NVMe block device driver + object store.
#![cfg(target_arch = "aarch64")]

use crate::{
    ipc::ConnectionRights,
    logln,
    service::{
        bootstrap,
        loader,
        supervisor::{
            self,
            DriverGrant,
            NameServiceHandle,
        },
    },
};

#[cfg(target_arch = "aarch64")]
const NS_ELF: &[u8] = include_bytes!("ns.elf");
#[cfg(target_arch = "aarch64")]
const NVME_ELF: &[u8] = include_bytes!("nvme.elf");
#[cfg(target_arch = "aarch64")]
const OBJSTORE_ELF: &[u8] = include_bytes!("objstore.elf");

#[cfg(target_arch = "aarch64")]
static mut TEST_STATE: Option<NameServiceHandle> = None;

pub fn test_el0_nvme() {
    #[cfg(target_arch = "aarch64")]
    {
        logln!("Testing EL0 userspace NVMe block device driver and object store...");
        let name_service = supervisor::spawn_name_service(NS_ELF, 0x4e414d45, 1, 8);
        unsafe { TEST_STATE = Some(name_service) };
        let _vtid =
            crate::cpu::scheduler::spawn_thread(crate::memory::KERNEL_ASID, verify_el0_nvme);
        logln!("[nvme] verifier deferred");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 NVMe + objstore test (AArch64 only).");
    }
}

fn wait_for_nvme() -> (usize, u32) {
    let topo = &crate::device_management::topology::DEVICE_TOPOLOGY;
    if let Some((bar0, irq)) =
        crate::device_management::drivers::busses::pci_express::topology::lookup_first_nvme(
            &topo.pcie,
        )
    {
        logln!("[nvme] PCI topology: BAR0={:#x} intid={}", bar0, irq);
        return (bar0 as usize, irq as u32);
    }
    let bar0: usize = 0x1000_0000;
    let intid: u32 = 44;
    logln!("[nvme] fallback: BAR0={:#x} intid={}", bar0, intid);
    (bar0, intid)
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_nvme() {
    let ns = unsafe { TEST_STATE.as_ref() }.expect("[nvme] test state missing");
    logln!("[nvme] verifier running, discovering NVMe...");
    let (bar0, intid) = wait_for_nvme();

    // --- Spawn NVMe driver ---
    let driver = supervisor::spawn_driver_with_name_service(
        NVME_ELF,
        ns,
        ConnectionRights::CALL,
        DriverGrant {
            mmio_phys_base: bar0,
            mmio_pages: 2,
            intid,
        },
    );
    let driver_cfg: *const u32 = {
        let base: *mut u8 = driver.status_frame.into();
        base as *const u32
    };
    logln!("[nvme] driver spawned (asid={})", driver.asid);

    // Wait for driver to register
    let sentinel_ptr: *const u32 = unsafe { driver_cfg.add(5) };
    while unsafe { core::ptr::read_volatile(sentinel_ptr) } != 0x900d {
        core::hint::spin_loop();
    }
    logln!("[nvme] driver ready");

    // --- Spawn object store with handoff ---
    let ep_cap: u64 = unsafe { core::ptr::read_volatile(driver_cfg.add(12) as *const u64) };
    let loaded = loader::load_domain(OBJSTORE_ELF);
    let handoff_conn = crate::ipc::connection_delegate(
        driver.asid,
        ep_cap,
        loaded.asid,
        ConnectionRights::SEND | ConnectionRights::CALL,
    )
    .expect("[nvme] handoff connection failed");
    let ns_conn = crate::ipc::connection_delegate(
        ns.domain.asid,
        ns.endpoint_cap,
        loaded.asid,
        ConnectionRights::CALL,
    )
    .expect("[nvme] ns connection failed");
    bootstrap::write_bootstrap_cap(loaded.config_frame, ns_conn);
    bootstrap::write_handoff_state(loaded.config_frame, 0, 0, handoff_conn);
    bootstrap::write_manifest(loaded.config_frame, &[]);
    let objstore = supervisor::start_domain(loaded);
    let obj_cfg: *const u32 = {
        let base: *mut u8 = objstore.status_frame.into();
        base as *const u32
    };
    logln!("[nvme] objstore spawned (asid={})", objstore.asid);

    // Wait for object store to register
    while unsafe { core::ptr::read_volatile(obj_cfg.add(1)) } != 0x900d {
        core::hint::spin_loop();
    }

    logln!("[nvme] SUCCESS: NVMe driver and object store both initialised and registered.");
}
