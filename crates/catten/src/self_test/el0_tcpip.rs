//! Self-test: TCP/IP over the smoltcp userspace adapter.
//!
//! Spawns the `tcpip` service (which becomes a frouter consumer for IPv4/ARP
//! frames and serves the socket API) and a `tcpclient` smoke program on each
//! guest. The client is self-configuring from its NIC MAC: the node with an
//! even last MAC octet listens and echoes, the other connects and sends; the
//! exchange proves ARP resolution, the TCP handshake, and data flow across
//! the two-node stream LAN.
//!
//! Requires the net driver and the frouter to be up (the net self-test spawns
//! both under `virtio_net_test` with the frouter enabled). Both QEMU guests
//! must run this test.
mod inner {
    use crate::{
        ipc::ConnectionRights,
        logln,
        service::supervisor::{
            self,
            NameServiceHandle,
            ServiceDomain,
        },
    };

    // "SENT" packed LE; written to tcpclient's named stage field on success.
    const CLIENT_SENTINEL: u32 = 0x5345_4e54;

    static mut TCPIP_NS: Option<NameServiceHandle> = None;

    fn spawn_binary(
        image: &[u8],
        ns: &NameServiceHandle,
        manifest: &[crate::service::bootstrap::ManifestEntry<'_>],
    ) -> ServiceDomain {
        supervisor::spawn_with_manifest(image, ns, ConnectionRights::CALL, manifest)
    }

    pub fn test_el0_tcpip() {
        logln!("Testing EL0 TCP/IP (smoltcp over the frouter)...");

        let name_service = supervisor::node_name_service();
        let ns_asid = name_service.domain.asid;
        logln!("[tcpip] using node name service (asid={})", ns_asid);

        unsafe { TCPIP_NS = Some(name_service) };

        let _vtid = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Tcpip,
            verify_el0_tcpip,
        );
        logln!("[tcpip] verifier deferred (waits for tcpip service + cross-node TCP exchange)");
    }

    extern "C" fn verify_el0_tcpip() {
        use crate::cpu::scheduler::yield_lp;

        let ns = unsafe { TCPIP_NS.as_ref() }.expect("[tcpip] test state missing");

        let tcpip = spawn_binary(
            crate::service::store::service_elf(b"tcpip").expect("[el0_tcpip] tcpip.elf"),
            ns,
            &[],
        );
        logln!("[tcpip] service spawned (asid={})", tcpip.asid);
        let tcpip_cfg: *const u8 = {
            let base: *mut u8 = tcpip.status_frame.into();
            base
        };

        // Wait for the tcpip service to enter its serving stage (6).
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        while unsafe {
            crate::self_test::status_u32(tcpip_cfg, charlotte_launch::tcpip_status::STAGE)
        } < 6
        {
            deadline.assert_pending("EL0 tcpip service startup");
            yield_lp();
        }
        logln!("[tcpip] service reached serving stage.");

        // Spawn the smoke client; it self-configures its role from the NIC MAC
        // and performs the cross-node TCP exchange.
        let client = spawn_binary(
            crate::service::store::service_elf(b"tcpclient").expect("[el0_tcpip] tcpclient.elf"),
            ns,
            &[],
        );
        let client_cfg: *const u8 = {
            let base: *mut u8 = client.status_frame.into();
            base
        };
        logln!("[tcpip] client spawned (asid={})", client.asid);

        let mut spins: u64 = 0;
        let deadline = crate::self_test::results::Deadline::after_millis(90_000);
        while unsafe {
            crate::self_test::status_u32(client_cfg, charlotte_launch::tcpclient_status::STAGE)
        } != CLIENT_SENTINEL
        {
            spins += 1;
            if spins.is_multiple_of(200_000) {
                let stage = unsafe {
                    crate::self_test::status_u32(
                        client_cfg,
                        charlotte_launch::tcpclient_status::STAGE,
                    )
                };
                let local_ip = unsafe {
                    crate::self_test::status_u32(
                        client_cfg,
                        charlotte_launch::tcpclient_status::LOCAL_IP,
                    )
                };
                let error = unsafe {
                    crate::self_test::status_u32(
                        client_cfg,
                        charlotte_launch::tcpclient_status::ERROR,
                    )
                };
                let rx_total = unsafe {
                    crate::self_test::status_u32(
                        tcpip_cfg,
                        charlotte_launch::tcpip_status::RX_TOTAL,
                    )
                };
                let tx_ok = unsafe {
                    crate::self_test::status_u32(tcpip_cfg, charlotte_launch::tcpip_status::TX_OK)
                };
                let sock_count = unsafe {
                    crate::self_test::status_u32(tcpip_cfg, charlotte_launch::tcpip_status::SOCKETS)
                };
                let frouter_base = crate::self_test::FROUTER_STATUS_FRAME
                    .load(core::sync::atomic::Ordering::Acquire)
                    as *const u8;
                let frouter_rx = if frouter_base.is_null() {
                    0
                } else {
                    unsafe {
                        crate::self_test::status_u32(
                            frouter_base,
                            charlotte_launch::frouter_status::RX_TOTAL,
                        )
                    }
                };
                let frouter_fwd = if frouter_base.is_null() {
                    0
                } else {
                    unsafe {
                        crate::self_test::status_u32(
                            frouter_base,
                            charlotte_launch::frouter_status::FORWARDED,
                        )
                    }
                };
                logln!(
                    "[tcpip] waiting: client stage={:#x} ip={:#x} error={:#x} tcpip rx={} tx={} \
                     sockets={} frouter rx={} fwd={}",
                    stage,
                    local_ip,
                    error,
                    rx_total,
                    tx_ok,
                    sock_count,
                    frouter_rx,
                    frouter_fwd
                );
            }
            deadline.assert_pending("EL0 tcpip cross-node exchange");
            yield_lp();
        }

        let local_ip = unsafe {
            crate::self_test::status_u32(client_cfg, charlotte_launch::tcpclient_status::LOCAL_IP)
        };
        let rx_total = unsafe {
            crate::self_test::status_u32(tcpip_cfg, charlotte_launch::tcpip_status::RX_TOTAL)
        };
        let tx_ok = unsafe {
            crate::self_test::status_u32(tcpip_cfg, charlotte_launch::tcpip_status::TX_OK)
        };
        logln!(
            "[tcpip] client succeeded (local ip {}.{}.{}.{}) tcpip rx={} tx={}",
            (local_ip >> 24) & 0xff,
            (local_ip >> 16) & 0xff,
            (local_ip >> 8) & 0xff,
            local_ip & 0xff,
            rx_total,
            tx_ok
        );
        assert!(
            rx_total >= 2 && tx_ok >= 1,
            "[tcpip] expected IP/ARP frames to flow through the frouter into tcpip"
        );

        logln!(
            "[tcpip] SUCCESS: two guests exchanged TCP data through the smoltcp adapter over the \
             Ethernet frouter."
        );
        crate::self_test::results::pass(crate::self_test::results::TestId::Tcpip);
    }
}

pub use inner::test_el0_tcpip;
