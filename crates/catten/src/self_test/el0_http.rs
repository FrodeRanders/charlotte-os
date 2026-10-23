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
    const DNS_OP_EVENT_FIRE: u32 = 12;
    // ns opcodes.
    const NS_OP_LOOKUP: u32 = 2;

    static mut HTTP_NS: Option<(NameServiceHandle, crate::memory::AddressSpaceHandle)> = None;

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

    fn fire_catalog_marker(kernel_conn: u64) -> Option<i64> {
        use crate::{
            ipc,
            memory::KERNEL_ASID,
        };
        // Serialized kernel fixture ABI boundary. Move the exact marker
        // payload once; dispose each observed returned capability after IPC.
        let memory = crate::memory::object::allocate_with_bytes(KERNEL_ASID, b"alpha").ok()?;
        let call = match ipc::scalar_call_with_memory_move(
            KERNEL_ASID,
            kernel_conn,
            DNS_OP_EVENT_FIRE,
            5,
            memory,
        ) {
            Ok(call) => call,
            Err(_) => {
                let _ = crate::memory::object::close_cap(KERNEL_ASID, memory);
                return None;
            }
        };
        let result = ipc::wait_reply(KERNEL_ASID, call)
            .ok()
            .and_then(|()| ipc::poll_reply(KERNEL_ASID, call).ok().flatten());
        let status = result.as_ref().map(|reply| reply.result);
        if let Some(reply) = result {
            if let Some(memory) = reply.memory {
                let _ = crate::memory::object::close_cap(KERNEL_ASID, memory);
            }
            if let Some(connection) = reply.cap {
                let _ = ipc::close_cap(KERNEL_ASID, connection);
            }
        }
        let _ = ipc::close_cap(KERNEL_ASID, call);
        status
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

    fn verify_ns_status(ns: &NameServiceHandle, root: crate::memory::AddressSpaceHandle) {
        use charlotte_protocol_ns::snapshot;

        use crate::{
            ipc,
            memory::object,
        };

        let caller = root.id();
        let kernel_ns = ipc::connection_delegate(
            ns.domain.asid,
            ns.endpoint_cap,
            caller,
            ipc::ConnectionRights::CALL,
        )
        .unwrap();

        // Serialized kernel fixture ABI: the captured live test namespace owns this
        // endpoint and every source through explicit post-observation disposal.
        // Neither raw fixture IDs nor status bytes authorize physical cleanup.
        fn finish(caller: crate::memory::AddressSpaceId, call: u64) -> crate::ipc::ReplyValue {
            assert!(ipc::wait_reply_timeout(caller, call, 5_000).unwrap());
            let reply = ipc::poll_reply(caller, call).unwrap().unwrap();
            ipc::close_cap(caller, call).unwrap();
            reply
        }
        fn scalar(
            caller: crate::memory::AddressSpaceId,
            kernel_ns: u64,
            opcode: u32,
            arg0: u64,
        ) -> i64 {
            let reply = finish(caller, ipc::scalar_call(caller, kernel_ns, opcode, arg0).unwrap());
            assert!(reply.cap.is_none() && reply.memory.is_none());
            reply.result
        }
        let endpoint = ipc::endpoint_create(caller, 0x5354_4154, 1, 1).unwrap();
        let source =
            ipc::connection_delegate(caller, endpoint, caller, ipc::ConnectionRights::ALL).unwrap();
        let mut name = [b'x'; charlotte_protocol_ns::MAX_NAME_LEN];
        let prefix = b"audit.ns.status.";
        name[..prefix.len()].copy_from_slice(prefix);
        let mut last_generation = 0;
        for index in 0..17 {
            name[255] = b'a' + index;
            let memory = object::allocate_with_bytes(caller, &name).unwrap();
            let reply = finish(
                caller,
                ipc::scalar_call_with_connection_copy(
                    caller,
                    kernel_ns,
                    3,
                    name.len() as u64,
                    source,
                    ipc::ConnectionRights::ALL,
                    memory,
                )
                .unwrap(),
            );
            assert!(reply.result > 0 && reply.cap.is_none() && reply.memory.is_none());
            last_generation = reply.result as u64;
            object::close_cap(caller, memory).unwrap();
        }
        let retire = charlotte_protocol_ns::encode(
            charlotte_protocol_ns::Action::Retire,
            charlotte_protocol_ns::Request {
                name: &name,
                generation: last_generation,
            },
        )
        .unwrap();
        let memory = object::allocate_with_bytes(caller, &retire).unwrap();
        let reply = finish(
            caller,
            ipc::scalar_call_with_memory_move(
                caller,
                kernel_ns,
                charlotte_protocol_ns::OP_UNREGISTER_GENERATION,
                retire.len() as u64,
                memory,
            )
            .unwrap(),
        );
        assert_eq!(reply.result, last_generation as i64);
        assert!(reply.cap.is_none() && reply.memory.is_none());

        assert_eq!(scalar(caller, kernel_ns, 9, 1), -2);
        let memory = object::allocate_with_bytes(caller, b"NSST").unwrap();
        let reply = finish(
            caller,
            ipc::scalar_call_with_connection_copy(
                caller,
                kernel_ns,
                9,
                0,
                source,
                ipc::ConnectionRights::ALL,
                memory,
            )
            .unwrap(),
        );
        assert_eq!(reply.result, -2);
        assert!(reply.cap.is_none() && reply.memory.is_none());
        assert!(!object::info(caller, memory).unwrap().lent);
        object::close_cap(caller, memory).unwrap();

        // Two accepted requests for the same missing name distinguish actual
        // pending-request count from the retired distinct-name count.
        let pending = [0; 2].map(|_| {
            ipc::scalar_call(caller, kernel_ns, 2, u64::from_le_bytes(*b"statwait")).unwrap()
        });
        let reply = finish(caller, ipc::scalar_call(caller, kernel_ns, 9, 0).unwrap());
        assert!(reply.cap.is_none());
        let memory = reply.memory.unwrap();
        let length = usize::try_from(reply.result).unwrap();
        assert!((4097..=snapshot::MAX_SNAPSHOT_LEN).contains(&length));
        let bytes = object::snapshot_bytes(caller, memory, length).unwrap();
        let observation = snapshot::decode(&bytes).expect("complete current NS status");
        assert!(observation.pending() >= 2);
        let rows: alloc::vec::Vec<_> = observation.records().collect();
        assert_eq!(observation.len(), rows.len());
        assert_eq!(rows.iter().filter(|row| row.name.starts_with(prefix)).count(), 16);
        for index in 0..16 {
            name[255] = b'a' + index;
            assert!(rows.iter().any(|row| row.name == name && row.generation > 0));
        }
        name[255] = b'q';
        assert!(!rows.iter().any(|row| row.name == name), "retired row must be absent");
        object::close_cap(caller, memory).unwrap();
        let reply = finish(
            caller,
            ipc::scalar_call_with_connection(
                caller,
                kernel_ns,
                1,
                u64::from_le_bytes(*b"statwait"),
                source,
                ipc::ConnectionRights::ALL,
            )
            .unwrap(),
        );
        assert!(reply.result > 0 && reply.cap.is_none() && reply.memory.is_none());
        for call in pending {
            let reply = finish(caller, call);
            assert!(reply.result > 0 && reply.memory.is_none());
            ipc::close_cap(caller, reply.cap.unwrap()).unwrap();
        }
        ipc::close_cap(caller, source).unwrap();
        ipc::close_cap(caller, kernel_ns).unwrap();
        // Keep the live endpoint in the fixture's captured namespace so the host
        // verifies the same full names in HTTP. Namespace teardown owns it.
        logln!(
            concat!(
                "[http] CNSSTAT1 complete multi-page status: {} rows, ",
                "16 full 256-byte names, retired row absent; ",
                "pending requests and invalid attachments checked"
            ),
            rows.len()
        );
    }

    pub fn test_el0_http() {
        logln!("Testing EL0 HTTP keyhole (hardcoded state server)...");

        let name_service = supervisor::node_name_service();
        let ns_asid = name_service.domain.asid;
        logln!("[http] using node name service (asid={})", ns_asid);

        // Opt-in trusted fixture preparation: a fresh ordinary root receives
        // one stable test principal and no manager/admin role before the
        // verifier publishes any requests. Existing boot policy is untouched.
        let fixture = crate::service::loader::create_user_address_space_handle();
        crate::memory::register_domain_authority(
            fixture,
            charlotte_launch::artifact_principal_id(b"ns-status-fixture"),
            0,
        )
        .unwrap();
        unsafe { HTTP_NS = Some((name_service, fixture)) };

        let _vtid = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Http,
            verify_el0_http,
        );
        logln!("[http] verifier deferred (waits for tcpip + httpd listening)");
    }

    extern "C" fn verify_el0_http() {
        use crate::cpu::scheduler::yield_lp;

        let (ns, fixture) = unsafe { HTTP_NS.as_ref() }.expect("[http] test state missing");

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
                        if let Some(result) = fire_catalog_marker(dns_conn) {
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
        verify_ns_status(ns, *fixture);
        logln!("[http] httpd reached the listening stage.");

        logln!("[http] SUCCESS: httpd is listening on port 80; host may curl the hostfwd port.");
        crate::self_test::results::pass(crate::self_test::results::TestId::Http);
    }
}

pub use inner::test_el0_http;
