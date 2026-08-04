//! Self-test: cluster administration service (`clusterctl`) and the serial
//! admin console.
//!
//! Spawns the embedded `clusterctl` EL0 service, which wraps the dns manifest
//! ops and the object store behind admin operations. The verifier drives the
//! programmatic flow through the clusterctl endpoint: upload a signed artifact
//! ("hello"), deploy it to a fixed node, query the manifest, and run the key
//! ceremony that commits the cluster's public key to replicated state. A kernel
//! thread then runs the interactive serial console (commands: `help`,
//! `upload`, `deploy`, `status`) on top of the same service.
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

    const CTL_ELF: &[u8] =
        include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/clusterctl.elf"));

    // "ctl" packed LE; catten_services::clusterctl.
    const CTL_NAME: u64 = 0x0000_0000_006c_7463;
    const CTL_OP_UPLOAD: u32 = 1;
    const CTL_OP_DEPLOY: u32 = 2;
    const CTL_OP_STATUS: u32 = 3;
    const CTL_OP_KEYCEREMONY: u32 = 4;
    const CTL_OP_KEY: u32 = 5;
    // "hello" packed LE.
    const HELLO_NAME: u64 = 0x0000_006f_6c6c_6568;
    // catten_services::dns::artifact_object_id(b"hello").
    const HELLO_OBJECT_ID: u64 = 0xfffe_d846_80aa_bd0b;
    // The pre-signed hello artifact (Ed25519 signature || payload), produced
    // off-cluster with tools/cluster-sign and the cluster's private key. The
    // cluster stores it as-is; nodes validate it against the public key.
    const HELLO_ARTIFACT: &[u8] = &[
        0x61, 0xd0, 0xe1, 0xd0, 0x32, 0xe8, 0x65, 0x9f, 0xe5, 0xd7, 0x38, 0x13, 0x6e, 0x22, 0xf8,
        0x99, 0x8f, 0x57, 0x22, 0x9c, 0x50, 0x9a, 0xc9, 0xb0, 0x25, 0x92, 0x63, 0x00, 0xa6, 0x61,
        0xd6, 0xd8, 0xd9, 0xdf, 0xf0, 0x19, 0xaa, 0xd5, 0x4c, 0xbf, 0xfd, 0x24, 0x90, 0xc4, 0xde,
        0x65, 0x6f, 0x51, 0x15, 0xb2, 0xac, 0xd2, 0x27, 0x6d, 0xf7, 0x50, 0x77, 0x08, 0x61, 0xa4,
        0xeb, 0x37, 0xf8, 0x0f, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x2d, 0x63, 0x6c, 0x75, 0x73, 0x74,
        0x65, 0x72,
    ];
    // The pre-signed greet artifact, for the console's `upload greet` (which
    // re-stages the signed blob rather than an unsigned payload).
    const GREET_ARTIFACT: &[u8] = &[
        0x6b, 0x60, 0x4a, 0x5c, 0xee, 0x1c, 0x34, 0x16, 0x75, 0x38, 0x81, 0x3b, 0x36, 0xba, 0x00,
        0xfb, 0x73, 0x35, 0x3d, 0x96, 0x62, 0x47, 0x4b, 0x89, 0x6a, 0xb1, 0x5c, 0x5d, 0x98, 0x51,
        0x4b, 0x3c, 0x87, 0x23, 0xc7, 0x13, 0xde, 0xe6, 0x3c, 0xb3, 0x0d, 0x01, 0x3c, 0x64, 0x6c,
        0x14, 0x13, 0x76, 0x3b, 0x5d, 0xff, 0x3a, 0x93, 0x6a, 0x64, 0x93, 0x84, 0x11, 0x8f, 0x3b,
        0xe6, 0x08, 0xad, 0x0c, 0x63, 0x6c, 0x75, 0x73, 0x74, 0x65, 0x72, 0x2d, 0x67, 0x72, 0x65,
        0x65, 0x74, 0x69, 0x6e, 0x67, 0x2d, 0x76, 0x31,
    ];
    const CTL_STAGE_SERVING: u32 = 6;
    // ns opcodes.
    const NS_OP_LOOKUP: u32 = 2;
    // The fixed deployment target of the programmatic flow (guest B's node
    // key, FNV-1a of 52:54:00:12:34:02 truncated to 32 bits).
    const TARGET_NODE_KEY: u64 = 0x42e2_7737;

    static mut TEST_STATE: Option<NameServiceHandle> = None;
    /// The kernel-side connection to the clusterctl service.
    static mut CTL_CONN: u64 = 0;

    fn spawn_clusterctl(ns: &NameServiceHandle) -> ServiceDomain {
        let addr = crate::service::loader::load_domain(CTL_ELF);
        let conn = ipc::connection_delegate(
            ns.domain.asid,
            ns.endpoint_cap,
            addr.asid,
            ConnectionRights::CALL,
        )
        .expect("[clusterctl] test conn delegate");
        crate::service::bootstrap::write_bootstrap_cap(addr.config_frame, conn);
        crate::service::bootstrap::write_manifest(
            addr.config_frame,
            &[ManifestEntry {
                key: charlotte_launch::CLUSTER_KEY_MANIFEST_KEY,
                flags: 0,
                value: ManifestValue::Bytes(&charlotte_launch::CLUSTER_PUBLIC_KEY),
            }],
        );
        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(addr.entry_vaddr) };
        let tid = crate::cpu::scheduler::spawn_thread(addr.asid, entry);
        let generation = crate::cpu::scheduler::threads::MASTER_THREAD_TABLE
            .read()
            .get(tid)
            .expect("[clusterctl] thread missing after spawn")
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
        .expect("[clusterctl] kernel name-service connection")
    }

    fn lookup_service(kernel_ns: u64, name: u64) -> Option<u64> {
        let call = ipc::scalar_call(KERNEL_ASID, kernel_ns, NS_OP_LOOKUP, name).ok()?;
        ipc::wait_reply(KERNEL_ASID, call).ok()?;
        ipc::poll_reply(KERNEL_ASID, call).ok().flatten().map(|reply| reply.cap.unwrap_or(0))
    }

    /// Scalar call with a moved memory object; returns the reply result and
    /// any returned memory cap (for `OP_STATUS`).
    fn call_with_memory_reply(
        kernel_conn: u64,
        opcode: u32,
        arg0: u64,
        bytes: &[u8],
    ) -> Option<(i64, Option<u64>)> {
        let mem = crate::memory::object::allocate_with_bytes(KERNEL_ASID, bytes).ok()?;
        let call =
            ipc::scalar_call_with_memory_move(KERNEL_ASID, kernel_conn, opcode, arg0, mem).ok()?;
        ipc::wait_reply(KERNEL_ASID, call).ok()?;
        let reply = ipc::poll_reply(KERNEL_ASID, call).ok().flatten();
        let result = reply.map(|reply| (reply.result, reply.memory));
        let _ = ipc::close_cap(KERNEL_ASID, call);
        result
    }

    /// Read up to `len` bytes from a moved memory object cap in the kernel
    /// address space, then release it.
    fn read_moved_memory(cap: u64, base: usize, len: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        if crate::memory::object::map(KERNEL_ASID, cap, base.into(), false).is_ok() {
            for index in 0..len {
                bytes.push(unsafe { core::ptr::read_volatile((base + index) as *const u8) });
            }
            let _ = crate::memory::object::unmap(KERNEL_ASID, cap);
        }
        let _ = crate::memory::object::close_cap(KERNEL_ASID, cap);
        bytes
    }

    fn status_word(base: *const u8, offset: usize) -> u32 {
        unsafe { core::ptr::read_volatile(base.add(offset).cast::<u32>()) }
    }

    /// Pack a short name into a little-endian u64 (mirrors the userspace
    /// `catten_services::name` helper).
    fn pack_name(name: &[u8]) -> u64 {
        let mut packed = 0u64;
        for (index, byte) in name.iter().copied().enumerate().take(8) {
            packed |= u64::from(byte) << (8 * index);
        }
        packed
    }

    pub fn test_el0_clusterctl() {
        logln!("Testing cluster administration service (clusterctl + serial console)...");
        let name_service = supervisor::node_name_service();
        unsafe { TEST_STATE = Some(name_service) };
        let _verifier = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Clusterctl,
            verify_el0_clusterctl,
        );
        logln!("[clusterctl] verifier deferred (drives upload/deploy/status; console on serial)");
    }

    extern "C" fn verify_el0_clusterctl() {
        use crate::cpu::scheduler::yield_lp;

        let ns = unsafe { TEST_STATE.as_ref() }.expect("[clusterctl] test state missing");
        let kernel_ns = kernel_ns_connection(ns);

        let ctl = spawn_clusterctl(ns);
        logln!("[clusterctl] clusterctl spawned (asid={})", ctl.asid);
        let ctl_cfg: *const u8 = {
            let base: *mut u8 = ctl.status_frame.into();
            base
        };
        let bootstrap_deadline = crate::self_test::results::Deadline::after_millis(120_000);
        while status_word(ctl_cfg, 0) < CTL_STAGE_SERVING {
            bootstrap_deadline.assert_pending("EL0 clusterctl local bootstrap");
            yield_lp();
        }
        logln!("[clusterctl] clusterctl reached serving stage.");

        // Resolve the service connection for both the programmatic flow and
        // the serial console.
        let mut ctl_conn = 0u64;
        let lookup_deadline = crate::self_test::results::Deadline::after_millis(30_000);
        while ctl_conn == 0 {
            ctl_conn = lookup_service(kernel_ns, CTL_NAME).unwrap_or(0);
            if ctl_conn == 0 {
                lookup_deadline.assert_pending("EL0 clusterctl registration");
                yield_lp();
            }
        }
        unsafe { CTL_CONN = ctl_conn };

        // --- Programmatic flow through clusterctl ---
        // 1. Upload the pre-signed artifact (signature || payload). The cluster stores it as-is;
        //    validation happens at pickup.
        let mut request = Vec::with_capacity(8 + HELLO_ARTIFACT.len());
        request.extend_from_slice(&(HELLO_ARTIFACT.len() as u64).to_le_bytes());
        request.extend_from_slice(HELLO_ARTIFACT);
        let uploaded = call_with_memory_reply(ctl_conn, CTL_OP_UPLOAD, HELLO_NAME, &request);
        logln!("[clusterctl] upload result = {uploaded:?}");
        assert_eq!(
            uploaded.map(|(result, _)| result),
            Some(HELLO_OBJECT_ID as i64),
            "[clusterctl] upload must return the derived artifact object id"
        );

        // 2. Deploy the artifact to the fixed target node, best-effort: the leader's dns commits
        //    it; a follower's dns answers NOT_LEADER until (and unless) it wins an election. The
        //    status gate below is the real wait — the record replicates to every replica either
        //    way.
        let mut request = Vec::with_capacity(8);
        request.extend_from_slice(&TARGET_NODE_KEY.to_le_bytes());
        let mut deployed_generation: Option<i64> = None;

        // 3. Query the manifest until the deployment is present and assigned to the target node on
        //    every replica.
        let status_deadline = crate::self_test::results::Deadline::after_millis(120_000);
        let record;
        loop {
            // Best-effort deploy attempt (cheap when not the leader).
            if deployed_generation.is_none()
                && let Some((generation, _)) =
                    call_with_memory_reply(ctl_conn, CTL_OP_DEPLOY, HELLO_NAME, &request)
                && generation >= 1
            {
                deployed_generation = Some(generation);
            }

            let Some((status, memory)) =
                call_with_memory_reply(ctl_conn, CTL_OP_STATUS, HELLO_NAME, &[])
            else {
                status_deadline.assert_pending("EL0 clusterctl status");
                yield_lp();
                continue;
            };
            if let Some(memory) = memory.filter(|_| status >= 0) {
                record = read_moved_memory(memory, 0x0000_0000_0060_1000, 24);
                break;
            }
            status_deadline.assert_pending("EL0 clusterctl status replication");
            yield_lp();
        }
        logln!("[clusterctl] deploy result = {deployed_generation:?}");
        let generation = u64::from_le_bytes(record[0..8].try_into().expect("record generation"));
        let node_key = u64::from_le_bytes(record[16..24].try_into().expect("record node key"));
        logln!("[clusterctl] status: generation={} node={:#x}", generation, node_key);
        assert!(generation >= 1, "[clusterctl] deployed artifact must carry a manifest generation");
        assert_eq!(
            node_key, TARGET_NODE_KEY,
            "[clusterctl] deployment must be assigned to the requested node"
        );

        // --- Key ceremony ---
        // Commit the cluster's public key to the replicated state (the
        // manual establishment path) and read it back, exactly as a joining
        // node would: the key is obtained from the cluster, not from a
        // channel out of band. Best-effort like the deploy: the ceremony
        // commits through the leader's dns; the read gate below polls every
        // replica's locally applied state until it sees the key.
        let key_deadline = crate::self_test::results::Deadline::after_millis(120_000);
        let mut ceremony_committed = false;
        loop {
            if !ceremony_committed
                && let Some((generation, _)) = call_with_memory_reply(
                    ctl_conn,
                    CTL_OP_KEYCEREMONY,
                    0,
                    &charlotte_launch::CLUSTER_PUBLIC_KEY,
                )
                && generation >= 1
            {
                ceremony_committed = true;
                logln!("[clusterctl] key ceremony committed (generation {generation})");
            }
            let Some((status, memory)) = call_with_memory_reply(ctl_conn, CTL_OP_KEY, 0, &[])
            else {
                key_deadline.assert_pending("EL0 clusterctl key ceremony");
                yield_lp();
                continue;
            };
            if let Some(memory) = memory.filter(|_| status >= 0) {
                let key = read_moved_memory(memory, 0x0000_0000_0060_1000, 32);
                assert_eq!(
                    key.as_slice(),
                    &charlotte_launch::CLUSTER_PUBLIC_KEY,
                    "[clusterctl] the committed cluster key must match the injected key"
                );
                logln!("[clusterctl] cluster key committed and replicated across the cluster");
                break;
            }
            key_deadline.assert_pending("EL0 clusterctl key ceremony replication");
            yield_lp();
        }

        // --- Serial admin console ---
        // The console thread reads commands from the PL011 RX FIFO and calls
        // the same clusterctl service.
        crate::cpu::scheduler::spawn_thread(KERNEL_ASID, console_entry);
        logln!(
            "[clusterctl] SUCCESS: clusterctl uploaded, deployed, and reported the artifact; \
             serial console listening."
        );
        crate::self_test::results::pass(crate::self_test::results::TestId::Clusterctl);
    }

    // ---- serial admin console ------------------------------------------------

    /// Prompt printed at the start of each console line.
    const CONSOLE_PROMPT: &str = "\r\n[admin] ";

    /// The console thread entry: read serial lines and dispatch commands to
    /// the clusterctl service.
    extern "C" fn console_entry() {
        use crate::cpu::scheduler::yield_lp;

        logln!("[clusterctl] admin console ready (help for commands).");
        crate::log::serial::SERIAL.lock().write_bytes(CONSOLE_PROMPT);

        let mut line: Vec<u8> = Vec::new();
        loop {
            let byte = crate::log::serial::SERIAL.lock().try_get_byte();
            match byte {
                Some(byte) if byte == b'\r' || byte == b'\n' => {
                    if !line.is_empty() {
                        logln!("[admin] {}", core::str::from_utf8(&line).unwrap_or("<binary>"));
                        run_console_command(&line);
                        line.clear();
                        crate::log::serial::SERIAL.lock().write_bytes(CONSOLE_PROMPT);
                    }
                }
                Some(byte) if byte == 0x7f || byte == 0x08 => {
                    line.pop();
                }
                Some(byte) => {
                    line.push(byte);
                }
                None => yield_lp(),
            }
        }
    }

    fn run_console_command(line: &[u8]) {
        let mut parts =
            line.split(|byte| *byte == b' ' || *byte == b'\t').filter(|part| !part.is_empty());
        match parts.next() {
            Some(b"help") => {
                logln!(
                    "[admin] commands: upload <name> <payload> | deploy <name> <node-key-hex> | \
                     status <name>"
                );
            }
            Some(b"upload") => {
                let name = parts.next();
                let payload = parts.next();
                let Some(name) = name else {
                    logln!("[admin] usage: upload <name> [<payload>]");
                    return;
                };
                // `upload greet` (no payload) re-stages the pre-signed greet
                // artifact; anything else stores the given bytes as-is, so
                // only a payload signed off-cluster will validate at pickup.
                let artifact = match (name, payload) {
                    (b"greet", None) => GREET_ARTIFACT,
                    (_, None) => {
                        logln!(
                            "[admin] usage: upload <name> [<payload>] (no signed artifact for {})",
                            core::str::from_utf8(name).unwrap_or("?")
                        );
                        return;
                    }
                    (_, Some(payload)) => payload,
                };
                let mut request = Vec::with_capacity(8 + artifact.len());
                request.extend_from_slice(&(artifact.len() as u64).to_le_bytes());
                request.extend_from_slice(artifact);
                let conn = unsafe { CTL_CONN };
                let result = call_with_memory_reply(conn, CTL_OP_UPLOAD, pack_name(name), &request);
                match result.map(|(result, _)| result) {
                    Some(object_id) if object_id >= 0 => {
                        logln!(
                            "[admin] uploaded {} at object {object_id:#x}",
                            core::str::from_utf8(name).unwrap_or("?")
                        )
                    }
                    Some(error) => logln!("[admin] upload failed ({error})"),
                    None => logln!("[admin] upload call failed"),
                }
            }
            Some(b"deploy") => {
                let name = parts.next();
                let key = parts.next();
                let (Some(name), Some(key)) = (name, key) else {
                    logln!("[admin] usage: deploy <name> <node-key-hex>");
                    return;
                };
                let Ok(key_text) = core::str::from_utf8(key) else {
                    logln!("[admin] node key must be hex");
                    return;
                };
                let Ok(node_key) = u64::from_str_radix(key_text, 16) else {
                    logln!("[admin] node key must be hex");
                    return;
                };
                let mut request = Vec::with_capacity(8);
                request.extend_from_slice(&node_key.to_le_bytes());
                let conn = unsafe { CTL_CONN };
                let result = call_with_memory_reply(conn, CTL_OP_DEPLOY, pack_name(name), &request);
                match result.map(|(generation, _)| generation) {
                    Some(generation) if generation >= 1 => {
                        logln!(
                            "[admin] deployed {} (generation {generation})",
                            core::str::from_utf8(name).unwrap_or("?")
                        )
                    }
                    Some(error) => logln!("[admin] deploy failed ({error})"),
                    None => logln!("[admin] deploy call failed"),
                }
            }
            Some(b"status") => {
                let name = parts.next();
                let Some(name) = name else {
                    logln!("[admin] usage: status <name>");
                    return;
                };
                let conn = unsafe { CTL_CONN };
                let Some((status, memory)) =
                    call_with_memory_reply(conn, CTL_OP_STATUS, pack_name(name), &[])
                else {
                    logln!("[admin] status call failed");
                    return;
                };
                let Some(memory) = memory else {
                    logln!("[admin] status: {status}");
                    return;
                };
                let bytes = read_moved_memory(memory, 0x0000_0000_0060_1000, 32);
                let generation = u64::from_le_bytes(bytes[0..8].try_into().expect("generation"));
                let object_id = u64::from_le_bytes(bytes[8..16].try_into().expect("object id"));
                let node_key = u64::from_le_bytes(bytes[16..24].try_into().expect("node key"));
                logln!(
                    "[admin] {}: generation={generation} object={object_id:#x} node={node_key:#x}",
                    core::str::from_utf8(name).unwrap_or("?")
                );
            }
            _ => {
                logln!("[admin] unknown command (try 'help')");
            }
        }
    }
}

pub use inner::test_el0_clusterctl;
