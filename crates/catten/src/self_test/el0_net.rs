//! Self-test: Phase 9 userspace Ethernet driver.
//!
//! This module is invoked only with the `virtio_net_test` feature. The test
//! requires a supported PCI Ethernet function; starting it in the ordinary
//! disk-only QEMU configuration leaves
//! its deferred verifier waiting forever and keeps guest CPUs runnable.
//!
//! Uses the node name service; a deferred kernel verifier thread (which runs
//! after the scheduler and the topology probe
//! become active) discovers a virtio-net or Intel 82574L/E1000E device, grants
//! its BAR + IRQ to the driver domain, spawns a client that queries status,
//! and verifies the MAC and link state.

use crate::{
    ipc::ConnectionRights,
    logln,
    service::supervisor::{
        self,
        NameServiceHandle,
    },
};

#[cfg(not(feature = "relmsg_net_test"))]
const CLIENT_SENTINEL: u32 = 0xc0de;
#[cfg(feature = "relmsg_net_test")]
const CLIENT_SENTINEL: u32 = 0xc0de_cafe;
static mut TEST_STATE: Option<NameServiceHandle> = None;

pub fn test_el0_net() {
    logln!("Testing EL0 userspace Ethernet driver...");

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

extern "C" fn verify_el0_net() {
    use crate::cpu::scheduler::yield_lp;

    let ns = unsafe { TEST_STATE.as_ref() }.expect("[net] test state missing");

    logln!("[net] verifier running, waiting for PCI topology...");
    let net_stack = crate::service::launch::steady_state()
        .network
        .expect("[net] network stack missing from steady state");
    let driver = net_stack.driver;
    let driver_config = driver.status_frame;
    let driver_asid = driver.asid;
    logln!(
        "[net] started userspace driver (asid={}) with MMIO, IRQ, and protected-DMA grants",
        driver_asid
    );
    let _driver = driver;

    #[cfg(any(feature = "relmsg_net_test", feature = "dns_net_test"))]
    let relmsg_config = {
        let relmsg = supervisor::spawn_with_name_service(
            crate::service::store::service_elf(b"relmsg").expect("[el0_net] relmsg.elf"),
            ns,
            ConnectionRights::CALL,
        );
        logln!("[relmsg] service spawned (asid={})", relmsg.asid);
        relmsg.status_frame
    };
    #[cfg(all(feature = "dns_net_test", not(feature = "relmsg_net_test")))]
    let _ = relmsg_config;

    logln!(
        "[frouter] frame demux spawned (asid={}, tid={})",
        net_stack.frouter.asid,
        net_stack.frouter.tid
    );
    let frouter_config = net_stack.frouter.status_frame;
    let frouter_base: *mut u8 = frouter_config.into();
    crate::self_test::FROUTER_STATUS_FRAME
        .store(frouter_base as usize, core::sync::atomic::Ordering::Release);
    {
        let frouter_status: *const u32 = {
            let base: *mut u8 = frouter_config.into();
            base.cast_const().cast()
        };
        let deadline = crate::self_test::results::Deadline::after_millis(30_000);
        let mut last_stage = u32::MAX;
        while unsafe { core::ptr::read_volatile(frouter_status) } < 4 {
            let stage = unsafe { core::ptr::read_volatile(frouter_status) };
            if stage != last_stage {
                logln!("[frouter] waiting for serving stage (current={stage})");
                last_stage = stage;
            }
            deadline.assert_pending("EL0 frame router startup");
            yield_lp();
        }
        logln!("[frouter] reached serving stage.");
    }

    let client = supervisor::spawn_with_name_service(
        crate::service::store::service_elf(b"nclient").expect("[el0_net] nclient.elf"),
        ns,
        ConnectionRights::CALL,
    );
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
            if spins.is_multiple_of(100_000) {
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
                    let receiver_base: *mut u8 = frouter_config.into();
                    let receiver_stage =
                        unsafe { core::ptr::read_volatile(receiver_base as *const u32) };
                    let forwarded = unsafe {
                        core::ptr::read_volatile((receiver_base as *const u8).add(8) as *const u32)
                    };
                    let rx_seen =
                        unsafe { core::ptr::read_volatile(driver_cfg.add(4) as *const u16) };
                    let tx_seen =
                        unsafe { core::ptr::read_volatile(driver_cfg.add(4).add(2) as *const u16) };
                    let device_status =
                        unsafe { core::ptr::read_volatile(driver_cfg.add(5) as *const u8) };
                    let tx_avail = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(22) as *const u16)
                    };
                    let rx_unrecycled = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(44) as *const u16)
                    };
                    let rx_qsz = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(46) as *const u16)
                    };
                    let rx_pfn = unsafe { core::ptr::read_volatile(driver_cfg.add(6)) };
                    let tx_pfn = unsafe { core::ptr::read_volatile(driver_cfg.add(7)) };
                    let dma_faults = crate::device::fault_count();
                    let pending_faults = crate::device::pending_fault_events();
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
                         {} send {}/{} frouter {} forwarded {} client {} status={:#x} avail={} \
                         pfn={:#x}/{:#x} notify={}/{} enabled={}/{} dma-faults={}/{} rxq={}/{} \
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
                        rx_unrecycled,
                        rx_qsz,
                        client_error
                    );
                }
                #[cfg(not(feature = "relmsg_net_test"))]
                {
                    let rx_completed = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(16) as *const u16)
                    };
                    let tx_completed = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(18) as *const u16)
                    };
                    let driver_error = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(36) as *const u32)
                    };
                    let rx_unrecycled = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(44) as *const u16)
                    };
                    let rx_qsz = unsafe {
                        core::ptr::read_volatile((driver_cfg as *const u8).add(46) as *const u16)
                    };
                    logln!(
                        "[net] waiting: driver stage={} error={:#x} client stage={} rx/tx={}/{} \
                         queued-rx={}/{}",
                        ds,
                        driver_error,
                        cs,
                        rx_completed,
                        tx_completed,
                        rx_unrecycled,
                        rx_qsz
                    );
                }
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

    let tx_completed = {
        let deadline = crate::self_test::results::Deadline::after_millis(2_000);
        loop {
            let completed = unsafe {
                core::ptr::read_volatile((driver_cfg as *const u8).add(18) as *const u16)
            };
            if completed != 0 {
                break completed;
            }
            deadline.assert_pending("EL0 Ethernet transmit completion");
            yield_lp();
        }
    };

    let ds = unsafe { core::ptr::read_volatile(driver_cfg) };
    assert!(ds >= 6, "[net] driver must reach serving stage (got {})", ds);

    #[cfg(feature = "relmsg_net_test")]
    logln!(
        "[relmsg] SUCCESS: two guests exchanged sequenced, acknowledged Ethernet messages; \
         hardware TX completions={}.",
        tx_completed
    );
    #[cfg(not(feature = "relmsg_net_test"))]
    logln!(
        "[net] SUCCESS: NIC is online, link up, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, \
         hardware TX completions={}.",
        m0,
        m1,
        m2,
        m3,
        m4,
        m5,
        tx_completed
    );
    crate::self_test::results::pass(crate::self_test::results::TestId::Net);
}
