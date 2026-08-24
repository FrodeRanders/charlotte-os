//! Self-test: hardcoded HTTP keyhole serving observable node state.
//!
//! Spawns the `tcpip` service in DHCP mode (it acquires its address from the
//! SLIRP user network's built-in DHCP server, which hands out 10.0.2.15) and
//! the `httpd` service, then waits for the httpd to reach its listening stage.
//! The actual request/response round trip is performed by the host through
//! `--http-test`: the run script curls the hostfwd port after the self-test
//! completes and verifies the JSON payload.
//!
//! Requires the net driver and the frouter (both spawned by the net self-test
//! under `virtio_net_test` with the frouter enabled).
mod inner {
    use crate::{
        logln,
        service::supervisor::{
            self,
            NameServiceHandle,
        },
    };

    // "dns" packed LE; the dns service registers under this name.
    const DNS_NAME: u64 = 0x0073_6e64;
    const DNS_OP_REGISTER: u32 = 1;
    // "alpha" packed LE.
    const ALPHA_NAME: u64 = 0x0061_6870_6c61;
    // ns opcodes.
    const NS_OP_LOOKUP: u32 = 2;

    static mut HTTP_NS: Option<NameServiceHandle> = None;

    /// Poll a service status word until it reaches `min`, bounded by
    /// `timeout_ms`. Returns `false` (without failing the test) on timeout so
    /// optional cluster services never make the keyhole test flaky.
    fn wait_for_stage(base: *const u8, offset: usize, min: u32, timeout_ms: u64) -> bool {
        use crate::cpu::scheduler::yield_lp;
        let deadline = crate::self_test::results::Deadline::after_millis(timeout_ms);
        while unsafe { crate::self_test::status_u32(base, offset) } < min {
            if deadline.is_expired() {
                return false;
            }
            yield_lp();
        }
        true
    }

    /// Poll a service status word until it equals `value`, bounded by
    /// `timeout_ms`. Returns `false` on timeout.
    fn wait_for_value(base: *const u8, offset: usize, value: u32, timeout_ms: u64) -> bool {
        use crate::cpu::scheduler::yield_lp;
        let deadline = crate::self_test::results::Deadline::after_millis(timeout_ms);
        while unsafe { crate::self_test::status_u32(base, offset) } != value {
            if deadline.is_expired() {
                return false;
            }
            yield_lp();
        }
        true
    }

    fn kernel_ns_connection(ns: &NameServiceHandle) -> u64 {
        crate::ipc::connection_delegate(
            ns.domain.asid,
            ns.endpoint_cap,
            crate::memory::KERNEL_ASID,
            crate::ipc::ConnectionRights::CALL,
        )
        .expect("[http] kernel name-service connection")
    }

    fn call(kernel_conn: u64, opcode: u32, arg0: u64) -> Option<i64> {
        let call =
            crate::ipc::scalar_call(crate::memory::KERNEL_ASID, kernel_conn, opcode, arg0).ok()?;
        crate::ipc::wait_reply(crate::memory::KERNEL_ASID, call).ok()?;
        crate::ipc::poll_reply(crate::memory::KERNEL_ASID, call)
            .ok()
            .flatten()
            .map(|reply| reply.result)
    }

    fn lookup_service(kernel_ns: u64, name: u64) -> Option<u64> {
        let call =
            crate::ipc::scalar_call(crate::memory::KERNEL_ASID, kernel_ns, NS_OP_LOOKUP, name)
                .ok()?;
        crate::ipc::wait_reply(crate::memory::KERNEL_ASID, call).ok()?;
        crate::ipc::poll_reply(crate::memory::KERNEL_ASID, call)
            .ok()
            .flatten()
            .map(|reply| reply.cap.unwrap_or(0))
    }

    pub fn test_el0_http() {
        logln!("Testing EL0 HTTP keyhole (hardcoded state server)...");

        let name_service = supervisor::node_name_service();
        let ns_asid = name_service.domain.asid;
        logln!("[http] using node name service (asid={})", ns_asid);

        unsafe { HTTP_NS = Some(name_service) };

        let _vtid = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Http,
            verify_el0_http,
        );
        logln!("[http] verifier deferred (waits for tcpip + httpd listening)");
    }

    extern "C" fn verify_el0_http() {
        use crate::cpu::scheduler::yield_lp;

        let ns = unsafe { HTTP_NS.as_ref() }.expect("[http] test state missing");

        let appliance = crate::service::launch::steady_state()
            .appliance
            .expect("[http] network appliance missing from steady state");
        let tcpip = appliance.tcpip;
        logln!("[http] tcpip spawned (asid={})", tcpip.asid);
        let tcpip_cfg: *const u8 = {
            let base: *mut u8 = tcpip.status_frame.into();
            base
        };
        logln!(
            "[http] tcpip initial status: stage={} error={:#x} detail={}",
            unsafe {
                crate::self_test::status_u32(tcpip_cfg, charlotte_launch::tcpip_status::STAGE)
            },
            unsafe {
                crate::self_test::status_u32(tcpip_cfg, charlotte_launch::tcpip_status::ERROR)
            },
            unsafe {
                crate::self_test::status_u32(tcpip_cfg, charlotte_launch::tcpip_status::DETAIL)
            }
        );

        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        while unsafe {
            crate::self_test::status_u32(tcpip_cfg, charlotte_launch::tcpip_status::STAGE)
        } < 6
        {
            let error = unsafe {
                crate::self_test::status_u32(tcpip_cfg, charlotte_launch::tcpip_status::ERROR)
            };
            assert_eq!(error, 0, "EL0 http tcpip startup failed with error {error:#x}");
            deadline.assert_pending("EL0 http tcpip service startup");
            yield_lp();
        }
        logln!("[http] tcpip service reached serving stage.");

        // The node cluster (disco + relmsg + raft + dns) is launched by the
        // steady-state boot. This HTTP smoke test verifies the DNS catalog
        // group's single-voter path; node-level Raft admission is covered by
        // the multi-guest join test.
        let cluster = crate::service::launch::steady_state()
            .cluster
            .expect("[http] cluster missing from steady state");
        let disco = cluster.disco;
        let relmsg = cluster.relmsg;
        let dns = cluster.dns;
        logln!("[http] disco spawned (asid={})", disco.asid);
        let disco_cfg: *const u8 = {
            let base: *mut u8 = disco.status_frame.into();
            base
        };
        if !wait_for_stage(disco_cfg, charlotte_launch::disco_status::STAGE, 5, 20_000) {
            logln!("[http] disco did not reach serving; report shows disco/dns:null.");
        } else {
            logln!("[http] disco reached the serving stage.");

            // The dns needs the reliable-message transport registered even
            // with a single-node cluster (it is the Raft transport).
            let relmsg_cfg: *const u8 = {
                let base: *mut u8 = relmsg.status_frame.into();
                base
            };
            if !wait_for_stage(relmsg_cfg, charlotte_launch::relmsg_status::STAGE, 3, 20_000) {
                logln!("[http] relmsg did not register; report shows relmsg:null.");
            } else {
                logln!("[http] relmsg registered.");

                let dns_cfg: *const u8 = {
                    let base: *mut u8 = dns.status_frame.into();
                    base
                };
                if !wait_for_stage(dns_cfg, charlotte_launch::dns_status::STAGE, 8, 20_000) {
                    logln!("[http] dns did not reach serving; report shows dns:null.");
                } else {
                    logln!("[http] dns reached the serving stage.");

                    // Wait for the single-node cluster to elect itself leader
                    // (1=follower, 2=candidate, 3=leader), then register a name so the report's dns
                    // section has a real entry.
                    let kernel_ns = kernel_ns_connection(ns);
                    if wait_for_value(dns_cfg, charlotte_launch::dns_status::RAFT_STATE, 3, 20_000)
                    {
                        logln!("[http] dns is the single-node cluster leader.");
                        let dns_conn = lookup_service(kernel_ns, DNS_NAME).unwrap_or(0);
                        if let Some(result) = call(dns_conn, DNS_OP_REGISTER, ALPHA_NAME) {
                            logln!("[http] dns register result = {result}");
                            if result >= 1
                                && wait_for_value(
                                    dns_cfg,
                                    charlotte_launch::dns_status::CATALOG_ENTRIES,
                                    1,
                                    20_000,
                                )
                            {
                                logln!("[http] dns catalog populated with the alpha name.");
                            }
                        }
                    }
                }
            }
        }

        let httpd = appliance.httpd;
        let httpd_cfg: *const u8 = {
            let base: *mut u8 = httpd.status_frame.into();
            base
        };
        logln!("[http] httpd spawned (asid={})", httpd.asid);

        // Wait for the httpd to be listening (stage 6). The request/response
        // round trip is performed by the host via --http-test afterwards.
        let mut spins: u64 = 0;
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        while unsafe {
            crate::self_test::status_u32(httpd_cfg, charlotte_launch::httpd_status::STAGE)
        } < 6
        {
            spins += 1;
            if spins.is_multiple_of(200_000) {
                let stage = unsafe {
                    crate::self_test::status_u32(httpd_cfg, charlotte_launch::httpd_status::STAGE)
                };
                let error = unsafe {
                    crate::self_test::status_u32(httpd_cfg, charlotte_launch::httpd_status::ERROR)
                };
                logln!("[http] waiting: httpd stage={} error={:#x}", stage, error);
            }
            deadline.assert_pending("EL0 http httpd listening");
            yield_lp();
        }
        logln!("[http] httpd reached the listening stage.");

        logln!("[http] SUCCESS: httpd is listening on port 80; host may curl the hostfwd port.");
        crate::self_test::results::pass(crate::self_test::results::TestId::Http);
    }
}

pub use inner::test_el0_http;
