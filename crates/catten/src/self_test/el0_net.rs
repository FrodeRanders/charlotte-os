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
#[cfg(not(feature = "relmsg_net_test"))]
const NET_CLIENT_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/nclient.elf"));
#[cfg(feature = "relmsg_net_test")]
const NET_CLIENT_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/rclient.elf"));
#[cfg(feature = "relmsg_net_test")]
const RELMSG_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/relmsg.elf"));
#[cfg(feature = "relmsg_net_test")]
const RELRX_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/relrx.elf"));

#[cfg(target_arch = "aarch64")]
#[cfg(not(feature = "relmsg_net_test"))]
const CLIENT_SENTINEL: u32 = 0xc0de;
#[cfg(feature = "relmsg_net_test")]
const CLIENT_SENTINEL: u32 = 0xc0de_cafe;
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

        let _vtid = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Net,
            verify_el0_net,
        );
        logln!("[net] verifier deferred (waits for PCI topology + driver + client)");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 net driver test (AArch64 only).");
    }
}

fn wait_for_virtio_net() -> (usize, usize, u32, u32, Option<u64>) {
    // Device discovery finishes and publishes the immutable topology before
    // scheduler-driven verifiers run. Absence is therefore a configuration
    // error, not a condition on which this verifier should spin.
    let topology = &crate::device_management::topology::DEVICE_TOPOLOGY;
    let (bar0, pages, intid, requester_id, msi_address) =
        crate::device_management::drivers::busses::pci_express::topology::lookup_first_virtio_net(
            &topology.pcie,
        )
        .expect("[net] no virtio-net controller in the published PCI topology");
    logln!("[net] PCI topology: BAR0={:#x} intid={} requester={:#x}", bar0, intid, requester_id);
    (bar0 as usize & !0xfff, pages, intid, requester_id, msi_address)
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_net() {
    use crate::cpu::scheduler::yield_lp;

    let ns = unsafe { TEST_STATE.as_ref() }.expect("[net] test state missing");

    logln!("[net] verifier running, waiting for PCI topology...");
    let (bar0, mmio_pages, intid, requester_id, msi_address) = wait_for_virtio_net();
    let driver = supervisor::spawn_driver_with_name_service(
        NET_ELF,
        ns,
        ConnectionRights::CALL,
        DriverGrant {
            mmio_phys_base: bar0,
            mmio_pages,
            intid,
            dma_requester_id: Some(requester_id),
            dma_msi_address: msi_address,
        },
    );
    let driver_config = driver.status_frame;
    let driver_asid = driver.asid;
    logln!("[net] driver spawned (asid={}) with BAR0 + IRQ grants", driver_asid);
    let _driver = driver;

    #[cfg(feature = "relmsg_net_test")]
    let relmsg_config = {
        let relmsg = supervisor::spawn_with_name_service(RELMSG_ELF, ns, ConnectionRights::CALL);
        logln!("[relmsg] service spawned (asid={})", relmsg.asid);
        relmsg.status_frame
    };
    #[cfg(feature = "relmsg_net_test")]
    let receiver_config = {
        let receiver = supervisor::spawn_with_name_service(RELRX_ELF, ns, ConnectionRights::CALL);
        logln!("[relmsg] receive pump spawned (asid={})", receiver.asid);
        receiver.status_frame
    };

    let client = supervisor::spawn_with_name_service(NET_CLIENT_ELF, ns, ConnectionRights::CALL);
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
        #[cfg(not(feature = "relmsg_net_test"))]
        let deadline = crate::self_test::results::Deadline::after_millis(10_000);
        #[cfg(feature = "relmsg_net_test")]
        let deadline = crate::self_test::results::Deadline::after_millis(30_000);
        while unsafe { core::ptr::read_volatile(client_cfg) } != CLIENT_SENTINEL {
            spins += 1;
            if spins.is_multiple_of(2_000_000) {
                let ds = unsafe { core::ptr::read_volatile(driver_cfg) };
                #[cfg(not(feature = "relmsg_net_test"))]
                let cs = unsafe { core::ptr::read_volatile(client_cfg.add(3)) };
                #[cfg(feature = "relmsg_net_test")]
                let cs = unsafe { core::ptr::read_volatile(client_cfg) };
                #[cfg(feature = "relmsg_net_test")]
                {
                    let relmsg_base: *mut u8 = relmsg_config.into();
                    let rs = unsafe { core::ptr::read_volatile(relmsg_base as *const u32) };
                    let opcode =
                        unsafe { core::ptr::read_volatile(relmsg_base.add(4) as *const u32) };
                    let handled =
                        unsafe { core::ptr::read_volatile(relmsg_base.add(8) as *const u32) };
                    let relmsg_send =
                        unsafe { core::ptr::read_volatile(relmsg_base.add(12) as *const u32) };
                    let net_result =
                        unsafe { core::ptr::read_volatile(relmsg_base.add(16) as *const i64) };
                    let receiver_base: *mut u8 = receiver_config.into();
                    let receiver_stage =
                        unsafe { core::ptr::read_volatile(receiver_base as *const u32) };
                    let forwarded =
                        unsafe { core::ptr::read_volatile(receiver_base.add(4) as *const u32) };
                    let rx_seen =
                        unsafe { core::ptr::read_volatile(driver_cfg.add(4) as *const u16) };
                    let tx_seen =
                        unsafe { core::ptr::read_volatile(driver_cfg.add(4).add(2) as *const u16) };
                    let device_status =
                        unsafe { core::ptr::read_volatile(driver_cfg.add(5) as *const u8) };
                    let tx_avail = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(22) as *const u16)
                    };
                    let rx_pfn = unsafe { core::ptr::read_volatile(driver_cfg.add(6)) };
                    let tx_pfn = unsafe { core::ptr::read_volatile(driver_cfg.add(7)) };
                    let dma_faults = crate::device::smmu::fault_count();
                    let pending_faults = crate::device::smmu::pending_fault_events();
                    let driver_send = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(36) as *const u32)
                    };
                    let rx_notify = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(32) as *const u16)
                    };
                    let tx_notify = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(34) as *const u16)
                    };
                    let rx_enabled = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(40) as *const u16)
                    };
                    let tx_enabled = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(42) as *const u16)
                    };
                    let client_error = unsafe {
                        core::ptr::read_volatile((client_cfg as *const u8).add(8) as *const i64)
                    };
                    logln!(
                        "[net] waiting: driver {} rx/tx {}/{} send={} relmsg {} opcode {} handled \
                         {} send {}/{} relrx {} forwarded {} client {} status={:#x} avail={} \
                         pfn={:#x}/{:#x} notify={}/{} enabled={}/{} dma-faults={}/{} \
                         client-error={}",
                        ds,
                        rx_seen,
                        tx_seen,
                        driver_send,
                        rs,
                        opcode,
                        handled,
                        relmsg_send,
                        net_result,
                        receiver_stage,
                        forwarded,
                        cs,
                        device_status,
                        tx_avail,
                        rx_pfn,
                        tx_pfn,
                        rx_notify,
                        tx_notify,
                        rx_enabled,
                        tx_enabled,
                        dma_faults,
                        pending_faults,
                        client_error
                    );
                }
                #[cfg(not(feature = "relmsg_net_test"))]
                logln!("[net] waiting: driver stage {} client stage {}", ds, cs);
            }
            deadline.assert_pending("EL0 network client");
            yield_lp();
        }
    }

    let driver_bytes: *const u8 = driver_config.into();
    let link = unsafe { core::ptr::read_volatile(driver_bytes.add(12) as *const u16) } as u64;
    let m0 = unsafe { core::ptr::read_volatile(driver_bytes.add(4)) };
    let m1 = unsafe { core::ptr::read_volatile(driver_bytes.add(5)) };
    let m2 = unsafe { core::ptr::read_volatile(driver_bytes.add(6)) };
    let m3 = unsafe { core::ptr::read_volatile(driver_bytes.add(7)) };
    let m4 = unsafe { core::ptr::read_volatile(driver_bytes.add(8)) };
    let m5 = unsafe { core::ptr::read_volatile(driver_bytes.add(9)) };
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
    assert_ne!([m0, m1, m2, m3, m4, m5], [0; 6], "[net] MAC must be nonzero");
    assert_eq!(link, 1, "[net] link must be up");

    let ds = unsafe { core::ptr::read_volatile(driver_cfg) };
    assert!(ds >= 6, "[net] driver must reach serving stage (got {})", ds);

    #[cfg(feature = "relmsg_net_test")]
    logln!("[relmsg] SUCCESS: two guests exchanged sequenced, acknowledged Ethernet messages.");
    #[cfg(not(feature = "relmsg_net_test"))]
    logln!(
        "[net] SUCCESS: userspace virtio-net driver reached DRIVER_OK, read MAC \
         {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, and accepted an EL0 transmit request.",
        m0,
        m1,
        m2,
        m3,
        m4,
        m5
    );
    crate::self_test::results::pass(crate::self_test::results::TestId::Net);
}
