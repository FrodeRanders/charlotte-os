//! Self-test: hardcoded HTTP keyhole serving observable node state.
//!
//! Spawns the `tcpip` service configured for the SLIRP user network
//! (10.0.2.15, gateway 10.0.2.2) and the `httpd` service, then waits for the
//! httpd to reach its listening stage. The actual request/response round trip
//! is performed by the host through `--http-test`: the run script curls the
//! hostfwd port after the self-test completes and verifies the JSON payload.
//!
//! Requires the net driver and the frouter (both spawned by the net self-test
//! under `virtio_net_test` with the frouter enabled).
#![cfg(target_arch = "aarch64")]

mod inner {
    use crate::{
        ipc::ConnectionRights,
        logln,
        service::{
            bootstrap::{
                ManifestEntry,
                ManifestValue,
            },
            supervisor::{
                self,
                NameServiceHandle,
                ServiceDomain,
            },
        },
    };

    const IP_KEY: u64 = charlotte_launch::manifest_key(b"ip");
    const GATEWAY_KEY: u64 = charlotte_launch::manifest_key(b"gateway");
    const CLUSTER_KEY: u64 = charlotte_launch::manifest_key(b"cluster");
    const NODE_ID_KEY: u64 = charlotte_launch::manifest_key(b"node-id");
    const PEERS_KEY: u64 = charlotte_launch::manifest_key(b"peers");
    const ELECTION_KEY: u64 = charlotte_launch::manifest_key(b"elect-ms");

    // "dns" packed LE; the dns service registers under this name.
    const DNS_NAME: u64 = 0x0073_6e64;
    const DNS_OP_REGISTER: u32 = 1;
    // "alpha" packed LE.
    const ALPHA_NAME: u64 = 0x0061_6870_6c61;
    // ns opcodes.
    const NS_OP_LOOKUP: u32 = 2;

    const TCPIP_ELF: &[u8] =
        include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/tcpip.elf"));
    const HTTPD_ELF: &[u8] =
        include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/httpd.elf"));
    const DISCO_ELF: &[u8] =
        include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/disco.elf"));
    const DNS_ELF: &[u8] =
        include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/dns.elf"));
    const RELMSG_ELF: &[u8] =
        include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/relmsg.elf"));

    static mut HTTP_NS: Option<NameServiceHandle> = None;

    fn spawn_binary(
        image: &[u8],
        ns: &NameServiceHandle,
        manifest: &[ManifestEntry<'_>],
    ) -> ServiceDomain {
        let addr = crate::service::loader::load_domain(image);
        let conn = crate::ipc::connection_delegate(
            ns.domain.asid,
            ns.endpoint_cap,
            addr.asid,
            ConnectionRights::CALL,
        )
        .expect("http spawn conn delegate");
        crate::service::bootstrap::write_bootstrap_cap(addr.config_frame, conn);
        crate::service::bootstrap::write_manifest(addr.config_frame, manifest);
        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(addr.entry_vaddr) };
        let tid = crate::cpu::scheduler::spawn_thread(addr.asid, entry);
        let generation = crate::cpu::scheduler::threads::MASTER_THREAD_TABLE
            .read()
            .get(tid)
            .expect("http thread missing after spawn")
            .generation;
        ServiceDomain {
            asid: addr.asid,
            tid,
            generation,
            config_frame: addr.config_frame,
            status_frame: addr.status_frame,
        }
    }

    /// Poll a service status word until it reaches `min`, bounded by
    /// `timeout_ms`. Returns `false` (without failing the test) on timeout so
    /// optional cluster services never make the keyhole test flaky.
    fn wait_for_stage(word: *const u32, min: u32, timeout_ms: u64) -> bool {
        use crate::cpu::scheduler::yield_lp;
        let deadline = crate::self_test::results::Deadline::after_millis(timeout_ms);
        while unsafe { core::ptr::read_volatile(word) } < min {
            if deadline.is_expired() {
                return false;
            }
            yield_lp();
        }
        true
    }

    /// Poll a service status word until it equals `value`, bounded by
    /// `timeout_ms`. Returns `false` on timeout.
    fn wait_for_value(word: *const u32, value: u32, timeout_ms: u64) -> bool {
        use crate::cpu::scheduler::yield_lp;
        let deadline = crate::self_test::results::Deadline::after_millis(timeout_ms);
        while unsafe { core::ptr::read_volatile(word) } != value {
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

        let tcpip_manifest = [
            ManifestEntry {
                key: IP_KEY,
                flags: 0,
                value: ManifestValue::Bytes(&[10, 0, 2, 15]),
            },
            ManifestEntry {
                key: GATEWAY_KEY,
                flags: 0,
                value: ManifestValue::Bytes(&[10, 0, 2, 2]),
            },
        ];
        let tcpip = spawn_binary(TCPIP_ELF, ns, &tcpip_manifest);
        logln!("[http] tcpip spawned (asid={})", tcpip.asid);
        let tcpip_cfg: *const u32 = {
            let base: *mut u8 = tcpip.status_frame.into();
            base as *const u32
        };

        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        while unsafe { core::ptr::read_volatile(tcpip_cfg) } < 6 {
            deadline.assert_pending("EL0 http tcpip service startup");
            yield_lp();
        }
        logln!("[http] tcpip service reached serving stage.");

        // Stand up a single-node cluster (disco + dns) so the report can show
        // the distributed name service's catalog as the cluster view.
        let disco = spawn_binary(
            DISCO_ELF,
            ns,
            &[
                ManifestEntry {
                    key: NODE_ID_KEY,
                    flags: 0,
                    value: ManifestValue::Bytes(b"http-node"),
                },
                ManifestEntry {
                    key: CLUSTER_KEY,
                    flags: 0,
                    value: ManifestValue::Bytes(b"test-cluster"),
                },
            ],
        );
        logln!("[http] disco spawned (asid={})", disco.asid);
        let disco_cfg: *const u32 = {
            let base: *mut u8 = disco.status_frame.into();
            base as *const u32
        };
        if !wait_for_stage(disco_cfg, 5, 20_000) {
            logln!("[http] disco did not reach serving; report shows disco/dns:null.");
        } else {
            logln!("[http] disco reached the serving stage.");

            // The dns needs the reliable-message transport registered even
            // with a single-node cluster (it is the Raft transport).
            let relmsg = spawn_binary(RELMSG_ELF, ns, &[]);
            logln!("[http] relmsg spawned (asid={})", relmsg.asid);
            let relmsg_cfg: *const u32 = {
                let base: *mut u8 = relmsg.status_frame.into();
                base as *const u32
            };
            if !wait_for_stage(relmsg_cfg, 3, 20_000) {
                logln!("[http] relmsg did not register; report shows relmsg:null.");
            } else {
                logln!("[http] relmsg registered.");

                let dns = spawn_binary(
                    DNS_ELF,
                    ns,
                    &[
                        ManifestEntry {
                            key: CLUSTER_KEY,
                            flags: 0,
                            value: ManifestValue::Bytes(b"test-cluster"),
                        },
                        ManifestEntry {
                            key: PEERS_KEY,
                            flags: 0,
                            value: ManifestValue::Unsigned(1),
                        },
                        ManifestEntry {
                            key: ELECTION_KEY,
                            flags: 0,
                            value: ManifestValue::Unsigned(300),
                        },
                    ],
                );
                logln!("[http] dns spawned (asid={})", dns.asid);
                let dns_cfg: *const u32 = {
                    let base: *mut u8 = dns.status_frame.into();
                    base as *const u32
                };
                if !wait_for_stage(dns_cfg, 8, 20_000) {
                    logln!("[http] dns did not reach serving; report shows dns:null.");
                } else {
                    logln!("[http] dns reached the serving stage.");

                    // Wait for the single-node cluster to elect itself leader
                    // (word 6 on the dns status page: 1=follower, 2=candidate,
                    // 3=leader), then register a name so the report's dns
                    // section has a real entry.
                    let kernel_ns = kernel_ns_connection(ns);
                    if wait_for_value(unsafe { dns_cfg.add(6) }, 3, 20_000) {
                        logln!("[http] dns is the single-node cluster leader.");
                        let dns_conn = lookup_service(kernel_ns, DNS_NAME).unwrap_or(0);
                        if let Some(result) = call(dns_conn, DNS_OP_REGISTER, ALPHA_NAME) {
                            logln!("[http] dns register result = {result}");
                            if result == 0 && wait_for_value(unsafe { dns_cfg.add(7) }, 1, 20_000) {
                                logln!("[http] dns catalog populated with the alpha name.");
                            }
                        }
                    }
                }
            }
        }

        let httpd = spawn_binary(HTTPD_ELF, ns, &[]);
        let httpd_cfg: *const u32 = {
            let base: *mut u8 = httpd.status_frame.into();
            base as *const u32
        };
        logln!("[http] httpd spawned (asid={})", httpd.asid);

        // Wait for the httpd to be listening (stage 6). The request/response
        // round trip is performed by the host via --http-test afterwards.
        let mut spins: u64 = 0;
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        while unsafe { core::ptr::read_volatile(httpd_cfg) } < 6 {
            spins += 1;
            if spins.is_multiple_of(200_000) {
                let stage = unsafe { core::ptr::read_volatile(httpd_cfg) };
                let error = unsafe { core::ptr::read_volatile(httpd_cfg.add(2)) };
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
