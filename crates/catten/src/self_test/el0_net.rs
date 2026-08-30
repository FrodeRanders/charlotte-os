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
        let relmsg = crate::service::launch::steady_state()
            .cluster
            .expect("[net] operational cluster services missing")
            .relmsg;
        logln!("[relmsg] observing operational service (asid={})", relmsg.asid);
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
        let frouter_status: *const u8 = {
            let base: *mut u8 = frouter_config.into();
            base.cast_const()
        };
        let deadline = crate::self_test::results::Deadline::after_millis(30_000);
        let mut last_stage = u32::MAX;
        while unsafe {
            crate::self_test::status_u32(frouter_status, charlotte_launch::frouter_status::STAGE)
        } < 4
        {
            let stage = unsafe {
                crate::self_test::status_u32(
                    frouter_status,
                    charlotte_launch::frouter_status::STAGE,
                )
            };
            if stage != last_stage {
                logln!("[frouter] waiting for serving stage (current={stage})");
                last_stage = stage;
            }
            deadline.assert_pending("EL0 frame router startup");
            yield_lp();
        }
        logln!("[frouter] reached serving stage.");
    }

    #[cfg(feature = "relmsg_net_test")]
    let client_elf = crate::service::store::service_elf(b"rclient").expect("[el0_net] rclient.elf");
    #[cfg(not(feature = "relmsg_net_test"))]
    let client_elf = crate::service::store::service_elf(b"nclient").expect("[el0_net] nclient.elf");
    let client = supervisor::spawn_with_name_service(client_elf, ns, ConnectionRights::CALL);
    let client_config = client.status_frame;
    let client_asid = client.asid;
    logln!("[net] client spawned (asid={})", client_asid);
    let _client = client;

    let client_cfg: *const u8 = {
        let base: *mut u8 = client_config.into();
        base
    };
    let driver_cfg: *const u8 = {
        let base: *mut u8 = driver_config.into();
        base
    };

    // --- wait for client sentinel ---
    {
        let mut spins: u64 = 0;
        #[cfg(not(feature = "relmsg_net_test"))]
        let deadline = crate::self_test::results::Deadline::after_millis(10_000);
        #[cfg(feature = "relmsg_net_test")]
        let deadline = crate::self_test::results::Deadline::after_millis(30_000);
        while unsafe {
            crate::self_test::status_u32(client_cfg, charlotte_launch::net_client_status::SENTINEL)
        } != CLIENT_SENTINEL
        {
            spins += 1;
            if spins.is_multiple_of(100_000) {
                let ds = unsafe {
                    crate::self_test::status_u32(driver_cfg, charlotte_launch::net_status::STAGE)
                };
                #[cfg(not(feature = "relmsg_net_test"))]
                let cs = unsafe {
                    crate::self_test::status_u32(
                        client_cfg,
                        charlotte_launch::net_client_status::STAGE,
                    )
                };
                #[cfg(feature = "relmsg_net_test")]
                let cs = unsafe {
                    crate::self_test::status_u32(
                        client_cfg,
                        charlotte_launch::relmsg_client_status::STAGE,
                    )
                };
                #[cfg(feature = "relmsg_net_test")]
                {
                    let relmsg_base: *mut u8 = relmsg_config.into();
                    let rs = unsafe {
                        crate::self_test::status_u32(
                            relmsg_base,
                            charlotte_launch::relmsg_status::STAGE,
                        )
                    };
                    let opcode = unsafe {
                        crate::self_test::status_u32(
                            relmsg_base,
                            charlotte_launch::relmsg_status::LAST_OPCODE,
                        )
                    };
                    let handled = unsafe {
                        crate::self_test::status_u32(
                            relmsg_base,
                            charlotte_launch::relmsg_status::HANDLED,
                        )
                    };
                    let relmsg_send = unsafe {
                        crate::self_test::status_u32(
                            relmsg_base,
                            charlotte_launch::relmsg_status::RECEIVER_STAGE,
                        )
                    };
                    let net_result = unsafe {
                        crate::self_test::status_i64(
                            relmsg_base,
                            charlotte_launch::relmsg_status::LAST_SEND_RESULT,
                        )
                    };
                    let receiver_base: *mut u8 = frouter_config.into();
                    let receiver_stage = unsafe {
                        crate::self_test::status_u32(
                            receiver_base,
                            charlotte_launch::frouter_status::STAGE,
                        )
                    };
                    let forwarded = unsafe {
                        crate::self_test::status_u32(
                            receiver_base,
                            charlotte_launch::frouter_status::FORWARDED,
                        )
                    };
                    let rx_seen = unsafe {
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::RX_USED_SEEN,
                        )
                    };
                    let tx_seen = unsafe {
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::TX_USED_SEEN,
                        )
                    };
                    let device_status = unsafe {
                        core::ptr::read_volatile(
                            driver_cfg.add(charlotte_launch::net_status::DEVICE_STATUS),
                        )
                    };
                    let tx_avail = unsafe {
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::TX_AVAILABLE,
                        )
                    };
                    let rx_unrecycled = unsafe {
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::RX_UNRECYCLED,
                        )
                    };
                    let rx_qsz = unsafe {
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::RX_QUEUE_SIZE,
                        )
                    };
                    let rx_pfn = unsafe {
                        crate::self_test::status_u32(
                            driver_cfg,
                            charlotte_launch::net_status::RX_RING_PFN,
                        )
                    };
                    let tx_pfn = unsafe {
                        crate::self_test::status_u32(
                            driver_cfg,
                            charlotte_launch::net_status::TX_RING_PFN,
                        )
                    };
                    let dma_faults = crate::device::fault_count();
                    let pending_faults = crate::device::pending_fault_events();
                    let driver_send = unsafe {
                        crate::self_test::status_u32(
                            driver_cfg,
                            charlotte_launch::net_status::TX_PROGRESS,
                        )
                    };
                    let rx_notify = unsafe {
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::RX_NOTIFY,
                        )
                    };
                    let tx_notify = unsafe {
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::TX_NOTIFY,
                        )
                    };
                    let rx_enabled = unsafe {
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::RX_QUEUE_ENABLED,
                        )
                    };
                    let tx_enabled = unsafe {
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::TX_QUEUE_ENABLED,
                        )
                    };
                    let client_error = unsafe {
                        crate::self_test::status_i64(
                            client_cfg,
                            charlotte_launch::relmsg_client_status::SEND_RESULT,
                        )
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
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::RX_USED_SEEN,
                        )
                    };
                    let tx_completed = unsafe {
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::TX_USED_SEEN,
                        )
                    };
                    let driver_error = unsafe {
                        crate::self_test::status_u32(
                            driver_cfg,
                            charlotte_launch::net_status::TX_PROGRESS,
                        )
                    };
                    let rx_unrecycled = unsafe {
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::RX_UNRECYCLED,
                        )
                    };
                    let rx_qsz = unsafe {
                        crate::self_test::status_u16(
                            driver_cfg,
                            charlotte_launch::net_status::RX_QUEUE_SIZE,
                        )
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
    let link =
        unsafe { crate::self_test::status_u16(driver_bytes, charlotte_launch::net_status::LINK) }
            as u64;
    let mut mac = [0; 6];
    for (index, octet) in mac.iter_mut().enumerate() {
        *octet = unsafe {
            core::ptr::read_volatile(driver_bytes.add(charlotte_launch::net_status::MAC + index))
        };
    }
    let [m0, m1, m2, m3, m4, m5] = mac;
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
                crate::self_test::status_u16(driver_cfg, charlotte_launch::net_status::TX_USED_SEEN)
            };
            if completed != 0 {
                break completed;
            }
            deadline.assert_pending("EL0 Ethernet transmit completion");
            yield_lp();
        }
    };

    let ds =
        unsafe { crate::self_test::status_u32(driver_cfg, charlotte_launch::net_status::STAGE) };
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
