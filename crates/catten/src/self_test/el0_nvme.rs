//! Self-test: Phase 1 userspace NVMe block device driver.
//!
//! Spawns the name service synchronously during self-tests; a deferred kernel
//! verifier thread discovers the NVMe PCI device, grants its BAR0 + IRQ to
//! the driver domain, spawns a client that writes a pattern and reads it back,
//! and verifies the round-trip.
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
const NS_ELF: &[u8] = include_bytes!("ns.elf");
#[cfg(target_arch = "aarch64")]
const NVME_ELF: &[u8] = include_bytes!("nvme.elf");
#[cfg(target_arch = "aarch64")]
const NVME_CLIENT_ELF: &[u8] = include_bytes!("nvme_client.elf");
#[cfg(target_arch = "aarch64")]
const OBJSTORE_ELF: &[u8] = include_bytes!("objstore.elf");

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
const MAX_SPINS: u64 = 80_000_000;

#[cfg(target_arch = "aarch64")]
static mut TEST_STATE: Option<NameServiceHandle> = None;

pub fn test_el0_nvme() {
    #[cfg(target_arch = "aarch64")]
    {
        logln!("Testing EL0 userspace NVMe block device driver...");
        let name_service = supervisor::spawn_name_service(NS_ELF, NS_INTERFACE, 1, 8);
        let ns_asid = name_service.domain.asid;
        logln!("[nvme] name service spawned (asid={})", ns_asid);
        unsafe { TEST_STATE = Some(name_service) };
        let _vtid =
            crate::cpu::scheduler::spawn_thread(crate::memory::KERNEL_ASID, verify_el0_nvme);
        logln!("[nvme] verifier deferred (waits for PCI topology + driver + client)");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 NVMe driver test (AArch64 only).");
    }
}

/// Discover the NVMe BAR0 and INTID. On TCG, uses PCI topology scan.
/// On HVF (where ECAM doesn't work), falls back to hardcoded QEMU virt values.
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
    logln!("[nvme] verifier running, waiting for PCI topology...");
    let (bar0, intid) = wait_for_nvme();

    let driver = supervisor::spawn_driver_with_name_service(
        NVME_ELF,
        ns,
        ConnectionRights::CALL,
        DriverGrant {
            mmio_phys_base: bar0,
            mmio_pages: 2, // BAR0 registers at 0x0000 + doorbells at 0x1000
            intid,
        },
    );
    let driver_config = driver.status_frame;
    let driver_asid = driver.asid;
    logln!("[nvme] driver spawned (asid={}) with BAR0 + IRQ grants", driver_asid);

    let client = supervisor::spawn_with_name_service(NVME_CLIENT_ELF, ns, ConnectionRights::CALL);
    logln!("[nvme] client spawned (asid={})", client.asid);

    let driver_cfg_u32: *const u32 = {
        let base: *mut u8 = driver_config.into();
        base as *const u32
    };

    let ds = unsafe { core::ptr::read_volatile(driver_cfg_u32) };
    let ds_sub = unsafe { core::ptr::read_volatile(driver_cfg_u32.add(1)) };
    logln!("[nvme] driver initial stage={} sub={}", ds, ds_sub);

    let mut spins: u64 = 0;
    let mut last_stage: u32 = 0;
    let sentinel_ptr: *const u32 = unsafe { driver_cfg_u32.add(5) };
    while unsafe { core::ptr::read_volatile(sentinel_ptr) } != 0x900d {
        spins += 1;
        let ds = unsafe { core::ptr::read_volatile(driver_cfg_u32) };
        if ds != last_stage || spins % 500_000 == 0 {
            let ds_sub = unsafe { core::ptr::read_volatile(driver_cfg_u32.add(1)) };
            let sf_sq: u32 = unsafe { core::ptr::read_volatile(driver_cfg_u32.add(4)) };
            let raw_dw3: u32 = unsafe { core::ptr::read_volatile(driver_cfg_u32.add(11)) };
            let test_sf: u32 = unsafe { core::ptr::read_volatile(driver_cfg_u32.add(12)) };
            let rdw0: u32 = unsafe { core::ptr::read_volatile(driver_cfg_u32.add(13)) };
            let rdw3: u64 =
                unsafe { core::ptr::read_volatile(driver_cfg_u32.add(14) as *const u64) };
            let rdw5_lo: u32 = unsafe { core::ptr::read_volatile(driver_cfg_u32.add(16)) };
            let rdw5_hi: u32 = unsafe { core::ptr::read_volatile(driver_cfg_u32.add(17)) };
            logln!(
                "[nvme] waiting: stage={} sub={} sf_sq={} dw3={:#x} feat={} rdw0={:#x} rdw3={:#x} \
                 dw5={:#x}_{:08x} (spins={})",
                ds,
                ds_sub,
                sf_sq,
                raw_dw3,
                test_sf,
                rdw0,
                rdw3,
                rdw5_hi,
                rdw5_lo,
                spins
            );
            last_stage = ds;
        }
        assert!(spins < MAX_SPINS, "[nvme] FAILED waiting for nvme_client");
        core::hint::spin_loop();
    }

    logln!("[nvme] driver ready, spawning object store...");

    let objstore = supervisor::spawn_with_name_service(OBJSTORE_ELF, ns, ConnectionRights::CALL);
    let obj_cfg: *const u32 = {
        let base: *mut u8 = objstore.status_frame.into();
        base as *const u32
    };
    logln!("[nvme] objstore spawned (asid={}), waiting for sentinel...", objstore.asid);

    spins = 0;
    while unsafe { core::ptr::read_volatile(obj_cfg.add(1)) } != 0x900d {
        spins += 1;
        let stage = unsafe { core::ptr::read_volatile(obj_cfg) };
        let conn = unsafe { core::ptr::read_volatile(obj_cfg.add(2)) };
        if spins % 500_000 == 0 {
            let msgcnt = unsafe { core::ptr::read_volatile(driver_cfg_u32.add(12)) };
            let drv_stage = unsafe { core::ptr::read_volatile(driver_cfg_u32) };
            let drv_sub = unsafe { core::ptr::read_volatile(driver_cfg_u32.add(1)) };
            let sf_sq = unsafe { core::ptr::read_volatile(driver_cfg_u32.add(4)) };
            let obj_info = unsafe { core::ptr::read_volatile(obj_cfg.add(3)) };
            let obj_bs = unsafe { core::ptr::read_volatile(obj_cfg.add(4)) };
            let obj_tb = unsafe { core::ptr::read_volatile(obj_cfg.add(5)) };
            let blk_conn = unsafe { core::ptr::read_volatile(obj_cfg.add(6)) };
            logln!("[nvme] obj stage={} conn={:#x} blk={} info={:#x} bs={} tb={} | drv stage={} sub={} (spins={})",
                stage, conn, blk_conn, obj_info, obj_bs, obj_tb, drv_stage, drv_sub, spins);
        }
        core::hint::spin_loop();
    }
    logln!("[nvme] SUCCESS: NVMe driver and object store both initialised and registered.");
}
