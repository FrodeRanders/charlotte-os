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
    const DNS_RESULT_LOCAL: i64 = 0;
    const DNS_RESULT_REMOTE: i64 = 1;
    const DNS_ERR_NOT_FOUND: i64 = -1;
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

    static mut TEST_STATE: Option<NameServiceHandle> = None;

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

    fn call_with_memory(kernel_conn: u64, opcode: u32, arg0: u64, bytes: &[u8]) -> Option<i64> {
        let mem = crate::memory::object::allocate_with_bytes(KERNEL_ASID, bytes).ok()?;
        let call =
            ipc::scalar_call_with_memory_move(KERNEL_ASID, kernel_conn, opcode, arg0, mem).ok()?;
        ipc::wait_reply(KERNEL_ASID, call).ok()?;
        ipc::poll_reply(KERNEL_ASID, call).ok().flatten().map(|reply| reply.result)
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

    extern "C" fn verify_el0_dns() {
        use crate::cpu::scheduler::yield_lp;

        let ns = unsafe { TEST_STATE.as_ref() }.expect("[dns] test state missing");
        let kernel_ns = kernel_ns_connection(ns);

        // The cluster discovery service is spawned by the disco self-test
        // (disco_net_test is implied by dns_net_test); the dns service waits on
        // its registration for the peer set.
        fn fnv1a(bytes: &[u8]) -> u64 {
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for byte in bytes {
                hash ^= *byte as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            hash
        }
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
                value: ManifestValue::Unsigned(400),
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

        // Wait for the dns replica to enter its reactor (stage 8).
        let dns_cfg: *const u32 = {
            let base: *mut u8 = dns.status_frame.into();
            base as *const u32
        };
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        while unsafe { core::ptr::read_volatile(dns_cfg) } < 8 {
            deadline.assert_pending("EL0 dns startup");
            yield_lp();
        }
        logln!("[dns] replica reached serving stage.");

        // The Raft membership must include the peer discovered through the
        // cluster discovery service; a single-node cluster would silently pass
        // the register step without proving network replication.
        let peers = unsafe { core::ptr::read_volatile(dns_cfg.add(2)) };
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
            state = unsafe { core::ptr::read_volatile(dns_cfg.add(6)) };
            catalog = unsafe { core::ptr::read_volatile(dns_cfg.add(7)) };
            if catalog >= 1 || state == 3 {
                break;
            }
            spins += 1;
            if spins.is_multiple_of(2_000_000) {
                let stage = unsafe { core::ptr::read_volatile(dns_cfg) };
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
            assert_eq!(register, Some(0), "[dns] register must commit on the leader");
        } else {
            logln!("[dns] replica is a follower; waiting for replicated catalog.");
        }

        // Wait for the catalog to contain the entry (replicated everywhere).
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        loop {
            catalog = unsafe { core::ptr::read_volatile(dns_cfg.add(7)) };
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
        let echo = crate::service::supervisor::spawn_with_name_service(
            ECHO_ELF,
            ns,
            ConnectionRights::CALL,
        );
        logln!("[dns] echo spawned (asid={})", echo.asid);
        let _echo = echo;

        let is_leader = state == 3;
        if is_leader {
            // Wait for the local echo to register, then publish it.
            let deadline = crate::self_test::results::Deadline::after_millis(30_000);
            while lookup_service(kernel_ns, ECHO_NAME).unwrap_or(0) == 0 {
                deadline.assert_pending("EL0 dns local echo registration");
                yield_lp();
            }
            let register = call(dns_conn, DNS_OP_REGISTER, ECHO_NAME);
            logln!("[dns] echo publish result = {register:?}");
            assert_eq!(register, Some(0), "[dns] echo publish must commit");
        }

        // Wait for the catalog to carry both names.
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        loop {
            catalog = unsafe { core::ptr::read_volatile(dns_cfg.add(7)) };
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
        if is_leader {
            let deadline = crate::self_test::results::Deadline::after_millis(60_000);
            while unsafe { core::ptr::read_volatile(dns_cfg.add(9)) } == 0 {
                deadline.assert_pending("EL0 dns follower remote invocation");
                yield_lp();
            }
            logln!("[dns] leader served the follower's remote invocation.");
        }

        logln!(
            "[dns] SUCCESS: Raft-elected name service replicated the catalog and served a remote \
             invocation."
        );
        crate::self_test::results::pass(crate::self_test::results::TestId::Dns);
    }
}

pub use inner::test_el0_dns;
