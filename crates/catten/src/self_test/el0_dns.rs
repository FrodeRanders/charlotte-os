//! Self-test: distributed name service over Raft + discovery.
//!
//! Spawns the cluster discovery service and a `dns` replica, waits for the
//! two-node Raft group to elect a leader over the network, registers a name
//! through the leader, and verifies the name replicates to every replica's
//! catalog (cross-node). Both QEMU guests must run this test.
#![cfg(target_arch = "aarch64")]

mod inner {
    use alloc::vec::Vec;

    use crate::{
        ipc::{
            self,
            ConnectionRights,
        },
        logln,
        memory::KERNEL_ASID,
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

    const CLUSTER_KEY: u64 = charlotte_launch::manifest_key(b"cluster");
    const PEERS_KEY: u64 = charlotte_launch::manifest_key(b"peers");
    const MEMBER_KEY: u64 = charlotte_launch::manifest_key(b"member");
    const ELECTION_KEY: u64 = charlotte_launch::manifest_key(b"elect-ms");

    // "dns" packed LE; the dns service registers under this name.
    const DNS_NAME: u64 = 0x0073_6e64;
    // catten_services::dns opcodes.
    const DNS_OP_REGISTER: u32 = 1;
    const DNS_OP_LOOKUP: u32 = 2;
    const DNS_OP_CALL: u32 = 5;
    const DNS_OP_UNREGISTER: u32 = 7;
    const DNS_RESULT_LOCAL: i64 = 0;
    const DNS_RESULT_REMOTE: i64 = 1;
    const DNS_ERR_NOT_FOUND: i64 = -1;
    const DNS_ERR_STALE_GENERATION: i64 = -7;
    // "alpha" packed LE.
    const ALPHA_NAME: u64 = 0x0061_6870_6c61;
    // "echo" packed LE.
    const ECHO_NAME: u64 = 0x0000_6f68_6365;
    // catten_services::echo opcodes.
    const ECHO_OP_ECHO: u32 = 1;
    // ns opcodes.
    const NS_OP_LOOKUP: u32 = 2;

    const DNS_ELF: &[u8] =
        include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/dns.elf"));
    const ECHO_ELF: &[u8] =
        include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/echo.elf"));
    #[cfg(feature = "deploy_net_test")]
    const AGENT_ELF: &[u8] =
        include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/agent.elf"));

    static mut TEST_STATE: Option<NameServiceHandle> = None;

    /// FNV-1a (the same hash the node-identity scheme uses to derive member
    /// names from NIC MACs).
    fn fnv1a(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    fn spawn_with_manifest(
        image: &[u8],
        ns: &NameServiceHandle,
        manifest: &[ManifestEntry<'_>],
    ) -> ServiceDomain {
        let addr = crate::service::loader::load_domain(image);
        let conn = ipc::connection_delegate(
            ns.domain.asid,
            ns.endpoint_cap,
            addr.asid,
            ConnectionRights::CALL,
        )
        .expect("dns test conn delegate");
        crate::service::bootstrap::write_bootstrap_cap(addr.config_frame, conn);
        crate::service::bootstrap::write_manifest(addr.config_frame, manifest);
        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(addr.entry_vaddr) };
        let tid = crate::cpu::scheduler::spawn_thread(addr.asid, entry);
        let generation = crate::cpu::scheduler::threads::MASTER_THREAD_TABLE
            .read()
            .get(tid)
            .expect("dns thread missing after spawn")
            .generation;
        ServiceDomain {
            asid: addr.asid,
            address_space: addr.address_space,
            tid,
            generation,
            config_frame: addr.config_frame,
            status_frame: addr.status_frame,
        }
    }

    fn kernel_ns_connection(ns: &NameServiceHandle) -> u64 {
        ipc::connection_delegate(
            ns.domain.asid,
            ns.endpoint_cap,
            KERNEL_ASID,
            ConnectionRights::CALL,
        )
        .expect("[dns] kernel name-service connection")
    }

    fn call(kernel_conn: u64, opcode: u32, arg0: u64) -> Option<i64> {
        let call = ipc::scalar_call(KERNEL_ASID, kernel_conn, opcode, arg0).ok()?;
        ipc::wait_reply(KERNEL_ASID, call).ok()?;
        ipc::poll_reply(KERNEL_ASID, call).ok().flatten().map(|reply| reply.result)
    }

    fn status_word(base: *const u8, offset: usize) -> u32 {
        debug_assert_eq!(offset % core::mem::align_of::<u32>(), 0);
        unsafe { core::ptr::read_volatile(base.add(offset).cast::<u32>()) }
    }

    fn call_with_memory(kernel_conn: u64, opcode: u32, arg0: u64, bytes: &[u8]) -> Option<i64> {
        let mem = crate::memory::object::allocate_with_bytes(KERNEL_ASID, bytes).ok()?;
        let call =
            ipc::scalar_call_with_memory_move(KERNEL_ASID, kernel_conn, opcode, arg0, mem).ok()?;
        ipc::wait_reply(KERNEL_ASID, call).ok()?;
        let result = ipc::poll_reply(KERNEL_ASID, call).ok().flatten().map(|reply| reply.result);
        // Release the operation and its memory object so callers may retry in
        // a loop without exhausting the kernel heap.
        let _ = ipc::close_cap(KERNEL_ASID, call);
        result
    }

    fn lookup_service(kernel_ns: u64, name: u64) -> Option<u64> {
        let call = ipc::scalar_call(KERNEL_ASID, kernel_ns, NS_OP_LOOKUP, name).ok()?;
        ipc::wait_reply(KERNEL_ASID, call).ok()?;
        ipc::poll_reply(KERNEL_ASID, call).ok().flatten().map(|reply| reply.cap.unwrap_or(0))
    }

    pub fn test_el0_dns() {
        logln!("Testing distributed name service (Raft over the network)...");
        let name_service = supervisor::node_name_service();
        unsafe { TEST_STATE = Some(name_service) };
        let _verifier = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Dns,
            verify_el0_dns,
        );
        logln!("[dns] verifier deferred (waits for discovery + Raft election + replication)");
    }

    /// The cluster-deployment phase of the dns self-test (`deploy_net_test`):
    /// the leader deploys the `greet` artifact to the *other* node, the
    /// remote agent picks it up, verifies its cluster signature, registers the
    /// name, and serves it across the network; the leader then re-deploys to
    /// its own node and the service migrates (the old host retires, the new
    /// host takes over, generation fencing prevents the old host's unregister
    /// from clobbering the new registration).
    #[cfg(feature = "deploy_net_test")]
    fn run_deploy_phase(
        ns: &NameServiceHandle,
        is_leader: bool,
        dns_conn: u64,
        dns_cfg: *const u8,
    ) {
        use crate::cpu::scheduler::yield_lp;

        // "greet" packed LE (matches catten_services::deploy::NAME).
        const GREET_NAME: u64 = 0x0000_0074_6565_7267;
        // catten_services::dns::artifact_object_id(b"greet").
        const DEPLOY_OBJECT_ID: u64 = 0xfffe_ea29_637f_e28a;
        const DNS_OP_DEPLOY: u32 = 8;
        const DNS_OP_GET: u32 = 1;
        // catten_services::deploy::GREET_VALUE.
        const GREET_VALUE: i64 = 0x2d72_6574_7375_6c63;
        const AGENT_STAGE_IDENTITY: u32 = 2;
        const AGENT_STAGE_UPLOADED: u32 = 4;
        const AGENT_STAGE_SERVING: u32 = 6;
        const AGENT_STAGE_RETIRED: u32 = 7;

        logln!("[deploy] testing cluster deployment and migration...");

        // Spawn the local deploy agent. Its status page first publishes this
        // guest's node key (offset 16), then the uploaded stage. The launch
        // manifest carries the cluster's build-time public key, the agent's
        // bootstrap trust anchor for validating artifacts.
        let agent = spawn_with_manifest(
            AGENT_ELF,
            ns,
            &[ManifestEntry {
                key: charlotte_launch::CLUSTER_KEY_MANIFEST_KEY,
                flags: 0,
                value: ManifestValue::Bytes(&charlotte_launch::CLUSTER_PUBLIC_KEY),
            }],
        );
        logln!("[deploy] agent spawned (asid={})", agent.asid);
        let agent_cfg: *const u8 = {
            let base: *mut u8 = agent.status_frame.into();
            base
        };
        let deadline = crate::self_test::results::Deadline::after_millis(120_000);
        while status_word(agent_cfg, 0) < AGENT_STAGE_IDENTITY {
            deadline.assert_pending("EL0 deploy agent identity");
            yield_lp();
        }
        let my_key = status_word(agent_cfg, 16) as u64;
        let key_a = fnv1a(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x01]) & 0xffff_ffff;
        let key_b = fnv1a(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x02]) & 0xffff_ffff;
        let peer_key = if my_key == key_a {
            key_b
        } else {
            key_a
        };
        assert!(
            my_key == key_a || my_key == key_b,
            "[deploy] unexpected local node key {my_key:#x}"
        );
        logln!("[deploy] this node key = {my_key:#x}, peer = {peer_key:#x}");

        while status_word(agent_cfg, 0) < AGENT_STAGE_UPLOADED {
            deadline.assert_pending("EL0 deploy agent artifact upload");
            yield_lp();
        }
        logln!("[deploy] agent uploaded the artifact to the object store.");

        // The leader deploys the artifact to the peer node (cross-node
        // hosting is the point of the cluster).
        if is_leader {
            let mut request = Vec::with_capacity(16);
            request.extend_from_slice(&DEPLOY_OBJECT_ID.to_le_bytes());
            request.extend_from_slice(&peer_key.to_le_bytes());
            logln!(
                "[deploy] calling OP_DEPLOY on the peer (object {:#x} node {peer_key:#x})",
                DEPLOY_OBJECT_ID
            );
            let deploy = call_with_memory(dns_conn, DNS_OP_DEPLOY, GREET_NAME, &request);
            logln!("[deploy] peer deployment result = {deploy:?}");
            assert!(
                deploy.is_some_and(|generation| generation >= 1),
                "[deploy] peer deployment must return its committed generation"
            );
        }

        // The remote agent registers the deployed name; the catalog carries
        // alpha + greet on every replica.
        let deadline = crate::self_test::results::Deadline::after_millis(120_000);
        let mut gate_spins: u64 = 0;
        while status_word(dns_cfg, charlotte_launch::dns_status::CATALOG_ENTRIES) < 2 {
            gate_spins += 1;
            if gate_spins.is_multiple_of(2_000_000) {
                let stage = status_word(agent_cfg, 0);
                let catalog = status_word(dns_cfg, charlotte_launch::dns_status::CATALOG_ENTRIES);
                logln!(
                    "[deploy] waiting for remote registration: catalog={catalog} \
                     local-agent-stage={stage}"
                );
            }
            deadline.assert_pending("EL0 deploy remote registration");
            yield_lp();
        }
        logln!("[deploy] catalog carries the deployed name.");

        // Invoke the deployed service by name; the leader's call crosses the
        // network to the hosting peer.
        let mut request = Vec::with_capacity(12);
        request.extend_from_slice(&DNS_OP_GET.to_le_bytes());
        request.extend_from_slice(&0i64.to_le_bytes());
        let result = call_with_memory(dns_conn, DNS_OP_CALL, GREET_NAME, &request);
        logln!("[deploy] cross-node greet result = {result:?}");
        assert_eq!(
            result,
            Some(GREET_VALUE),
            "[deploy] cross-node invocation of the deployed artifact must return its payload value"
        );

        // The leader re-deploys the artifact to its own node: the service
        // migrates. The new host registers a fresh generation; the old host
        // retires and its generation-fenced unregister cannot clobber it.
        if is_leader {
            let mut request = Vec::with_capacity(16);
            request.extend_from_slice(&DEPLOY_OBJECT_ID.to_le_bytes());
            request.extend_from_slice(&my_key.to_le_bytes());
            let deploy = call_with_memory(dns_conn, DNS_OP_DEPLOY, GREET_NAME, &request);
            logln!("[deploy] migration deployment result = {deploy:?}");
            assert!(
                deploy.is_some_and(|generation| generation >= 2),
                "[deploy] migration deployment must return a newer generation"
            );
        }

        // After migration the local agent serves only if this node is the
        // leader (the new host); otherwise it must have retired.
        let deadline = crate::self_test::results::Deadline::after_millis(120_000);
        let expected_stage = if is_leader {
            AGENT_STAGE_SERVING
        } else {
            AGENT_STAGE_RETIRED
        };
        while status_word(agent_cfg, 0) != expected_stage {
            deadline.assert_pending("EL0 deploy migration handover");
            yield_lp();
        }
        logln!("[deploy] local agent stage {} after migration.", status_word(agent_cfg, 0));

        // The deployed service must still be reachable after the handover.
        // The old host retires before the new host registers, so a call can
        // land in the gap; retry until the name resolves to the new host.
        let deadline = crate::self_test::results::Deadline::after_millis(120_000);
        let post_migration;
        loop {
            let mut request = Vec::with_capacity(12);
            request.extend_from_slice(&DNS_OP_GET.to_le_bytes());
            request.extend_from_slice(&0i64.to_le_bytes());
            let result = call_with_memory(dns_conn, DNS_OP_CALL, GREET_NAME, &request);
            if result == Some(GREET_VALUE) {
                post_migration = result;
                break;
            }
            deadline.assert_pending("EL0 deploy post-migration reachability");
            crate::cpu::scheduler::sleep_millis(250);
            yield_lp();
        }
        logln!("[deploy] post-migration greet result = {post_migration:?}");
        assert_eq!(
            post_migration,
            Some(GREET_VALUE),
            "[deploy] the deployed artifact must remain reachable across migration"
        );

        // Barrier: keep this guest alive until the follower has *acknowledged*
        // a post-migration reply. The runner kills this QEMU the moment
        // SELFTEST COMPLETE prints; if the leader finished before the
        // follower's verification reply was delivered, the follower would
        // lose its peer mid-verification. Serving the call is not enough --
        // the reply must have reached the follower's transport.
        if is_leader {
            let acked_deadline = crate::self_test::results::Deadline::after_millis(120_000);
            let acks_before = status_word(dns_cfg, charlotte_launch::dns_status::REMOTE_CALL_ACKS);
            while status_word(dns_cfg, charlotte_launch::dns_status::REMOTE_CALL_ACKS)
                == acks_before
            {
                acked_deadline.assert_pending("EL0 deploy follower reply acknowledged");
                yield_lp();
            }
            logln!("[deploy] leader's post-migration reply was acknowledged by the follower.");
        }

        logln!(
            "[deploy] SUCCESS: the cluster deployed a signed artifact to the peer node, served it \
             across the network, and migrated it without losing the name."
        );
    }

    extern "C" fn verify_el0_dns() {
        use crate::cpu::scheduler::yield_lp;

        let ns = unsafe { TEST_STATE.as_ref() }.expect("[dns] test state missing");
        let kernel_ns = kernel_ns_connection(ns);

        // The cluster discovery service is spawned by the disco self-test
        // (disco_net_test is implied by dns_net_test); the dns service waits on
        // its registration for the peer set.
        let member_a = alloc::format!(
            "test-cluster:{:08x}",
            fnv1a(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x01]) & 0xffff_ffff
        )
        .into_bytes();
        let member_b = alloc::format!(
            "test-cluster:{:08x}",
            fnv1a(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x02]) & 0xffff_ffff
        )
        .into_bytes();
        let dns_manifest = [
            ManifestEntry {
                key: CLUSTER_KEY,
                flags: 0,
                value: ManifestValue::Bytes(b"test-cluster"),
            },
            ManifestEntry {
                key: PEERS_KEY,
                flags: 0,
                value: ManifestValue::Unsigned(2),
            },
            ManifestEntry {
                key: ELECTION_KEY,
                flags: 0,
                // Generous for the debug kernel behind the QEMU socketpair:
                // the network RTT is milliseconds-to-tens-of-milliseconds
                // under boot load, and an election timeout this short makes
                // leader elections race the transport.
                value: ManifestValue::Unsigned(2_000),
            },
            ManifestEntry {
                key: MEMBER_KEY,
                flags: 0,
                value: ManifestValue::Bytes(&member_a),
            },
            ManifestEntry {
                key: MEMBER_KEY,
                flags: 0,
                value: ManifestValue::Bytes(&member_b),
            },
        ];

        let dns = spawn_with_manifest(DNS_ELF, ns, &dns_manifest);
        logln!("[dns] dns spawned (asid={})", dns.asid);

        // Give each startup phase its own budget. In particular, do not spend
        // the Raft-initialisation budget while discovery is still waiting for
        // a peer on a slow emulator.
        let dns_cfg: *const u8 = {
            let base: *mut u8 = dns.status_frame.into();
            base
        };
        let bootstrap_deadline = crate::self_test::results::Deadline::after_millis(120_000);
        while status_word(dns_cfg, charlotte_launch::dns_status::STAGE) < 6 {
            bootstrap_deadline.assert_pending("EL0 dns local bootstrap");
            yield_lp();
        }
        let discovery_deadline = crate::self_test::results::Deadline::after_millis(120_000);
        while status_word(dns_cfg, charlotte_launch::dns_status::STAGE) < 7 {
            discovery_deadline.assert_pending("EL0 dns peer discovery");
            yield_lp();
        }
        let raft_init_deadline = crate::self_test::results::Deadline::after_millis(60_000);
        while status_word(dns_cfg, charlotte_launch::dns_status::STAGE) < 8 {
            raft_init_deadline.assert_pending("EL0 dns durable Raft initialisation");
            yield_lp();
        }
        logln!("[dns] replica reached serving stage.");

        // The Raft membership must include the peer discovered through the
        // cluster discovery service; a single-node cluster would silently pass
        // the register step without proving network replication.
        let peers = status_word(dns_cfg, charlotte_launch::dns_status::PEER_COUNT);
        logln!("[dns] Raft membership peers = {peers}");
        assert!(peers >= 2, "[dns] expected 2-node membership, discovered {peers}");

        // Resolve the dns endpoint. Either this replica becomes the Raft
        // leader (and registers a name) or the leader's registration
        // replicates here; both are reached via the network transport. Poll
        // the dns *status page* (state + catalog count) so the Raft clock in
        // the replica is not stalled by endpoint IPC wakeups.
        let mut dns_conn = 0u64;
        let mut spins: u64 = 0;
        let deadline = crate::self_test::results::Deadline::after_millis(90_000);
        let (mut state, mut catalog);
        loop {
            if dns_conn == 0 {
                dns_conn = lookup_service(kernel_ns, DNS_NAME).unwrap_or(0);
            }
            state = status_word(dns_cfg, charlotte_launch::dns_status::RAFT_STATE);
            catalog = status_word(dns_cfg, charlotte_launch::dns_status::CATALOG_ENTRIES);
            if catalog >= 1 || state == 3 {
                break;
            }
            spins += 1;
            if spins.is_multiple_of(2_000_000) {
                let stage = status_word(dns_cfg, charlotte_launch::dns_status::STAGE);
                logln!(
                    "[dns] waiting for leader or replication: stage={stage} state={state} \
                     catalog={catalog}"
                );
            }
            deadline.assert_pending("EL0 dns leader election / replication");
            yield_lp();
        }

        // If this replica is the leader, register a name for the cluster to
        // replicate.
        if state == 3 {
            logln!("[dns] replica is the Raft leader; registering name.");
            let register = call(dns_conn, DNS_OP_REGISTER, ALPHA_NAME);
            logln!("[dns] register result = {register:?}");
            assert!(
                register.is_some_and(|generation| generation >= 1),
                "[dns] register must return its committed generation"
            );
        } else {
            logln!("[dns] replica is a follower; waiting for replicated catalog.");
        }

        // Wait for the catalog to contain the entry (replicated everywhere).
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        loop {
            catalog = status_word(dns_cfg, charlotte_launch::dns_status::CATALOG_ENTRIES);
            if catalog >= 1 {
                logln!("[dns] catalog converged: {catalog} name(s).");
                break;
            }
            deadline.assert_pending("EL0 dns catalog replication");
            yield_lp();
        }

        // Look the name up: it must resolve to this node (local) or a remote
        // node; either proves the catalog entry replicated through the cluster.
        let lookup = call(dns_conn, DNS_OP_LOOKUP, ALPHA_NAME);
        logln!("[dns] lookup result = {lookup:?}");
        assert!(
            lookup == Some(DNS_RESULT_LOCAL) || lookup == Some(DNS_RESULT_REMOTE),
            "[dns] replicated lookup must resolve local or remote, got {lookup:?}"
        );
        assert_ne!(lookup, Some(DNS_ERR_NOT_FOUND), "[dns] replicated name must not be unknown");

        // ---- Remote invocation through the catalog ----
        // Host a local echo on every node; the leader publishes it through the
        // dns, so a client on either node can invoke it by name and the dns
        // routes to the hosting node over the network.
        // The service-lifecycle and NVMe persistent-upgrade suites also use
        // the global `echo` name. Wait until both have finished replacing and
        // tearing down their generations so this lifecycle probe has sole
        // ownership of the name it is about to publish.
        let echo_owner_deadline = crate::self_test::results::Deadline::after_millis(120_000);
        while !crate::self_test::results::has_passed(crate::self_test::results::TestId::Service)
            || !crate::self_test::results::has_passed(crate::self_test::results::TestId::Nvme)
        {
            echo_owner_deadline.assert_pending("EL0 dns waiting for echo-mutating suites");
            yield_lp();
        }
        let echo = crate::service::supervisor::spawn_with_name_service(
            ECHO_ELF,
            ns,
            ConnectionRights::CALL,
        );
        logln!("[dns] echo spawned (asid={})", echo.asid);

        let is_leader = state == 3;
        let mut echo_generation = 0;
        if is_leader {
            // A prior lifecycle test may have left a closed echo generation
            // in the local registry. The new domain's own serving stage is
            // the authoritative proof that it has replaced that entry; a
            // mere successful lookup could still return the stale generation.
            let deadline = crate::self_test::results::Deadline::after_millis(30_000);
            let echo_status: *const u32 = {
                let base: *mut u8 = echo.status_frame.into();
                base as *const u32
            };
            while unsafe { core::ptr::read_volatile(echo_status) } < 6 {
                deadline.assert_pending("EL0 dns new echo serving stage");
                yield_lp();
            }
            loop {
                let connection = lookup_service(kernel_ns, ECHO_NAME).unwrap_or(0);
                if connection != 0 {
                    ipc::close_cap(KERNEL_ASID, connection)
                        .expect("[dns] local echo lookup connection close");
                    break;
                }
                deadline.assert_pending("EL0 dns local echo registration");
                yield_lp();
            }
            let register = call(dns_conn, DNS_OP_REGISTER, ECHO_NAME);
            logln!("[dns] echo publish result = {register:?}");
            echo_generation = register.unwrap_or(0);
            assert!(echo_generation >= 1, "[dns] echo publish must return its generation");
        }

        // Wait for the catalog to carry both names.
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        loop {
            catalog = status_word(dns_cfg, charlotte_launch::dns_status::CATALOG_ENTRIES);
            if catalog >= 2 {
                logln!("[dns] catalog carries {catalog} name(s); invoking echo.");
                break;
            }
            deadline.assert_pending("EL0 dns echo publication");
            yield_lp();
        }

        // Invoke the echo service by name. If this node hosts it the dns calls
        // it locally; otherwise the dns relays the call to the hosting node.
        let mut request = Vec::new();
        request.extend_from_slice(&ECHO_OP_ECHO.to_le_bytes());
        request.extend_from_slice(&42i64.to_le_bytes());
        let echo_result = call_with_memory(dns_conn, DNS_OP_CALL, ECHO_NAME, &request);
        logln!("[dns] remote echo result = {echo_result:?}");
        assert_eq!(
            echo_result,
            Some(42),
            "[dns] cross-node invocation of echo must return the echoed value"
        );

        // The leader's own invocation is local. Keep the leader VM alive
        // until its DNS replica has also served the follower's remote call;
        // otherwise the runner can observe local SELFTEST COMPLETE and stop
        // the leader while the follower is still awaiting its reply.
        let mut query_barrier = 0;
        if is_leader {
            let deadline = crate::self_test::results::Deadline::after_millis(60_000);
            while status_word(dns_cfg, charlotte_launch::dns_status::REMOTE_CALL_ACKS) == 0 {
                deadline.assert_pending("EL0 dns follower remote invocation");
                yield_lp();
            }
            logln!("[dns] leader served the follower's remote invocation.");
            query_barrier =
                status_word(dns_cfg, charlotte_launch::dns_status::REMOTE_QUERY_REPLY_ACKS);

            assert_eq!(
                status_word(dns_cfg, charlotte_launch::dns_status::PUBLICATION_LIFECYCLE),
                1,
                "[dns] local endpoint-close watch must be installed before teardown"
            );
            assert_eq!(status_word(dns_cfg, charlotte_launch::dns_status::CATALOG_ENTRIES), 2);
            crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
                .read()
                .abort_thread(echo.tid)
                .expect("[dns] hosted echo abort");
            crate::service::supervisor::wait_domain_exit(&echo, 30_000);
            crate::service::supervisor::teardown_domain(echo);
            logln!("[dns] generation {echo_generation} endpoint closed; awaiting tombstone.");
        }

        // Keep both voters alive until the exact-generation tombstone has
        // replicated. This also prevents the follower's runner from removing
        // quorum while the leader is committing the unregister operation.
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        let mut unregister_spins = 0u64;
        while status_word(dns_cfg, charlotte_launch::dns_status::CATALOG_ENTRIES) != 1 {
            unregister_spins = unregister_spins.wrapping_add(1);
            if unregister_spins.is_multiple_of(2_000_000) {
                let lifecycle =
                    status_word(dns_cfg, charlotte_launch::dns_status::PUBLICATION_LIFECYCLE);
                let raft_state = status_word(dns_cfg, charlotte_launch::dns_status::RAFT_STATE);
                logln!(
                    "[dns] waiting for endpoint tombstone: lifecycle={} raft-state={} catalog={}",
                    lifecycle,
                    raft_state,
                    status_word(dns_cfg, charlotte_launch::dns_status::CATALOG_ENTRIES)
                );
            }
            deadline.assert_pending("EL0 dns generation-fenced unregister replication");
            yield_lp();
        }
        if is_leader {
            while status_word(dns_cfg, charlotte_launch::dns_status::REMOTE_QUERY_REPLY_ACKS)
                <= query_barrier
            {
                deadline.assert_pending("EL0 dns follower tombstone acknowledgement");
                yield_lp();
            }
        } else {
            let lookup = call(dns_conn, DNS_OP_LOOKUP, ECHO_NAME);
            assert_eq!(
                lookup,
                Some(DNS_ERR_NOT_FOUND),
                "[dns] post-tombstone lookup must observe the removal"
            );
        }
        if is_leader {
            let stale_unregister = call_with_memory(
                dns_conn,
                DNS_OP_UNREGISTER,
                ECHO_NAME,
                &(echo_generation as u64).to_le_bytes(),
            );
            assert_eq!(stale_unregister, Some(DNS_ERR_STALE_GENERATION));
            logln!(
                "[dns] endpoint death unregistered generation {}; stale replay rejected.",
                echo_generation
            );
        }

        #[cfg(feature = "deploy_net_test")]
        run_deploy_phase(ns, is_leader, dns_conn, dns_cfg);

        logln!(
            "[dns] SUCCESS: Raft-elected name service replicated the catalog, served a remote \
             invocation, and automatically fenced endpoint-death unregister by generation."
        );
        crate::self_test::results::pass(crate::self_test::results::TestId::Dns);
    }
}

pub use inner::test_el0_dns;
