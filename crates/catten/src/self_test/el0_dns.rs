//! Self-test: distributed name service over Raft + discovery.
//!
//! Observes the operational discovery and `dns` services, waits for the
//! two-node Raft group to elect a leader over the network, registers a name
//! through the leader, and verifies the name replicates to every replica's
//! catalog (cross-node). Both QEMU guests must run this test.

mod inner {
    use alloc::vec::Vec;

    use crate::{
        ipc::{
            self,
            ConnectionRights,
        },
        logln,
        memory::KERNEL_ASID,
        service::supervisor::{
            self,
            NameServiceHandle,
        },
    };

    // "dns" packed LE; the dns service registers under this name.
    const DNS_NAME: u64 = 0x0073_6e64;
    // catten_services::dns opcodes.
    const DNS_OP_REGISTER: u32 = 1;
    const LONG_ECHO_NAME: &[u8] = b"cluster.console.echo.primary.v1";
    const DNS_OP_EVENT_FIRE: u32 = 12;
    const DNS_OP_LOOKUP: u32 = 2;
    const DNS_OP_CALL: u32 = 5;
    const DNS_ERR_TOO_LARGE: i64 = -4;
    const DNS_ERR_BAD_OPCODE: i64 = -3;
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

    static mut TEST_STATE: Option<NameServiceHandle> = None;

    /// FNV-1a (the same hash the node-identity scheme uses to derive member
    /// names from NIC MACs).
    #[cfg(feature = "deploy_net_test")]
    fn fnv1a(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
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

    fn finish_fixture_call(call: u64) -> Option<i64> {
        // Serialized kernel ABI fixture: poll observes each returned capability
        // exactly once. Dispose unused returned owners after IPC unlock; this
        // is not a userspace adoption API or physical-cleanup retry.
        let reply = ipc::wait_reply(KERNEL_ASID, call)
            .ok()
            .and_then(|()| ipc::poll_reply(KERNEL_ASID, call).ok().flatten());
        let result = reply.as_ref().map(|reply| reply.result);
        if let Some(reply) = reply {
            if let Some(memory) = reply.memory {
                let _ = crate::memory::object::close_cap(KERNEL_ASID, memory);
            }
            if let Some(connection) = reply.cap {
                let _ = ipc::close_cap(KERNEL_ASID, connection);
            }
        }
        let _ = ipc::close_cap(KERNEL_ASID, call);
        result
    }

    fn call(kernel_conn: u64, opcode: u32, arg0: u64) -> Option<i64> {
        let call = ipc::scalar_call(KERNEL_ASID, kernel_conn, opcode, arg0).ok()?;
        finish_fixture_call(call)
    }

    fn status_word(base: *const u8, offset: usize) -> u32 {
        debug_assert_eq!(offset % core::mem::align_of::<u32>(), 0);
        let slot = unsafe { &*base.add(offset).cast::<core::sync::atomic::AtomicU32>() };
        slot.load(core::sync::atomic::Ordering::Acquire)
    }

    fn call_with_memory(kernel_conn: u64, opcode: u32, arg0: u64, bytes: &[u8]) -> Option<i64> {
        let mem = crate::memory::object::allocate_with_bytes(KERNEL_ASID, bytes).ok()?;
        let call =
            match ipc::scalar_call_with_memory_move(KERNEL_ASID, kernel_conn, opcode, arg0, mem) {
                Ok(call) => call,
                Err(_) => {
                    let _ = crate::memory::object::close_cap(KERNEL_ASID, mem);
                    return None;
                }
            };
        finish_fixture_call(call)
    }

    fn invoke_dns(connection: u64, name: &[u8], opcode: u32, arg: i64) -> Option<i64> {
        let request = charlotte_protocol_dns::encode(charlotte_protocol_dns::Request {
            name,
            opcode,
            arg,
        })
        .expect("DNS fixture request encoding");
        call_with_memory(connection, DNS_OP_CALL, request.len() as u64, &request)
    }

    fn generation_request(
        connection: u64,
        name: &[u8],
        generation: u64,
        action: charlotte_protocol_ns::Action,
    ) -> Option<i64> {
        let request = charlotte_protocol_ns::encode(
            action,
            charlotte_protocol_ns::Request {
                name,
                generation,
            },
        )
        .expect("NS generation fixture encoding");
        let opcode = match action {
            charlotte_protocol_ns::Action::Observe => {
                charlotte_protocol_ns::OP_PUBLICATION_GENERATION
            }
            charlotte_protocol_ns::Action::Retire => {
                charlotte_protocol_ns::OP_UNREGISTER_GENERATION
            }
        };
        call_with_memory(connection, opcode, request.len() as u64, &request)
    }

    fn lookup_service(kernel_ns: u64, name: u64) -> Option<u64> {
        let call = ipc::scalar_call(KERNEL_ASID, kernel_ns, NS_OP_LOOKUP, name).ok()?;
        ipc::wait_reply(KERNEL_ASID, call).ok()?;
        ipc::poll_reply(KERNEL_ASID, call).ok().flatten().map(|reply| reply.cap.unwrap_or(0))
    }

    /// Stage the deploy artifact into the object store: create the object at
    /// `object_id`, set its size, write the bytes, and flush. Mirrors the
    /// agent's former self-staging; the kernel does it for the demo so the
    /// artifact bytes can live kernel-side.
    #[cfg(feature = "deploy_net_test")]
    fn stage_deploy_artifact(
        ns: &NameServiceHandle,
        object_id: u64,
        bytes: &[u8],
    ) -> Result<(), ()> {
        const OBJSTORE_NAME: u64 = 0x0000_0000_006a_626f; // "obj" packed LE.
        const OBJ_OP_CREATE_AT: u32 = 8;
        const OBJ_OP_SET_SIZE: u32 = 9;
        const OBJ_OP_WRITE: u32 = 3;
        const OBJ_OP_FLUSH: u32 = 6;
        // charlotte-protocol-objstore v1 status values (the kernel does not
        // otherwise depend on the userspace protocol crate).
        const OBJ_ERR_OK: i64 = 0;
        const OBJ_ERR_EXISTS: i64 = 5;

        let kernel_ns = kernel_ns_connection(ns);
        let obj_conn = lookup_service(kernel_ns, OBJSTORE_NAME).expect("[deploy] objstore lookup");

        let call = ipc::scalar_call(KERNEL_ASID, obj_conn, OBJ_OP_CREATE_AT, object_id)
            .expect("[deploy] objstore create call");
        ipc::wait_reply(KERNEL_ASID, call).expect("[deploy] objstore create reply");
        let result = ipc::poll_reply(KERNEL_ASID, call)
            .expect("[deploy] objstore create poll")
            .expect("[deploy] objstore create replied");
        // The initial disk image now carries the signed service bundle,
        // including `greet`. Treat an existing artifact as an update target;
        // SET_SIZE + WRITE below atomically replaces its object contents.
        if result.result != OBJ_ERR_OK && result.result != OBJ_ERR_EXISTS {
            return Err(());
        }
        let _ = ipc::close_cap(KERNEL_ASID, call);

        let size_bytes = (bytes.len() as u64).to_le_bytes();
        let call = call_with_memory(obj_conn, OBJ_OP_SET_SIZE, object_id, &size_bytes)
            .expect("[deploy] objstore set-size result");
        if call != OBJ_ERR_OK {
            return Err(());
        }
        let call = call_with_memory(obj_conn, OBJ_OP_WRITE, object_id, bytes)
            .expect("[deploy] objstore write result");
        if call != OBJ_ERR_OK {
            return Err(());
        }
        let call = ipc::scalar_call(KERNEL_ASID, obj_conn, OBJ_OP_FLUSH, 0)
            .expect("[deploy] objstore flush");
        ipc::wait_reply(KERNEL_ASID, call).expect("[deploy] objstore flush reply");
        let result = ipc::poll_reply(KERNEL_ASID, call)
            .expect("[deploy] objstore flush poll")
            .expect("[deploy] objstore flush replied");
        let _ = ipc::close_cap(KERNEL_ASID, call);
        if result.result != OBJ_ERR_OK {
            return Err(());
        }
        Ok(())
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
    /// name, and executes it across the network; the leader then re-deploys to
    /// its own node and the service migrates (the old host retires, the new
    /// host takes over, generation fencing prevents the old host's unregister
    /// from clobbering the new registration).
    #[cfg(feature = "deploy_net_test")]
    fn wait_rollout_ready(connection: u64, generation: u64, node: u64) {
        let deadline = crate::self_test::results::Deadline::after_millis(120_000);
        loop {
            // Kernel-only moved-cap ABI adapter. No userspace owner is reconstructed;
            // poll observes the returned cap exactly once, then both owners close.
            let memory = crate::memory::object::allocate_with_bytes(KERNEL_ASID, b"greet").unwrap();
            let call =
                match ipc::scalar_call_with_memory_move(KERNEL_ASID, connection, 18, 5, memory) {
                    Ok(call) => call,
                    Err(error) => {
                        crate::memory::object::close_cap(KERNEL_ASID, memory).unwrap();
                        panic!("rollout query submission: {error:?}");
                    }
                };
            ipc::wait_reply(KERNEL_ASID, call).unwrap();
            let reply = ipc::poll_reply(KERNEL_ASID, call).unwrap().unwrap();
            ipc::close_cap(KERNEL_ASID, call).unwrap();
            let ready = if let Some(memory) = reply.memory {
                let bytes = crate::memory::object::snapshot_bytes(KERNEL_ASID, memory, 40);
                crate::memory::object::close_cap(KERNEL_ASID, memory).unwrap();
                bytes.is_ok_and(|bytes| {
                    reply.result == 40
                        && bytes[0] == 2
                        && u64::from_le_bytes(bytes[8..16].try_into().unwrap()) == generation
                        && u64::from_le_bytes(bytes[24..32].try_into().unwrap()) == node
                        && bytes[32..36] == [1, 0, 1, 0]
                })
            } else {
                false
            };
            if ready {
                assert_controller_status(generation, node);
                return;
            }
            // Catalog readiness has no subscription ABI in this raw boot fixture.
            // This bounded query does not authorize teardown; existing owners and
            // acknowledged/forced retirement receipts provide that proof below.
            deadline.assert_pending("exact signed deployment readiness");
            crate::cpu::scheduler::sleep_millis(250);
        }
    }

    /// Check the public controller's full-name query after exact readiness.
    /// This serialized kernel fixture observes each moved result once and
    /// closes its caps outside IPC; no user-root mapping or cleanup retry.
    #[cfg(feature = "deploy_net_test")]
    fn assert_controller_status(generation: u64, node: u64) {
        let ns = unsafe { TEST_STATE.as_ref() }.expect("DNS fixture name service");
        let ns_connection = kernel_ns_connection(ns);
        let ctl = lookup_service(ns_connection, 0x006c_7463).expect("controller connection");
        ipc::close_cap(KERNEL_ASID, ns_connection).unwrap();
        let memory = crate::memory::object::allocate_with_bytes(KERNEL_ASID, b"greet").unwrap();
        let call = match ipc::scalar_call_with_memory_move(KERNEL_ASID, ctl, 3, 5, memory) {
            Ok(call) => call,
            Err(error) => {
                crate::memory::object::close_cap(KERNEL_ASID, memory).unwrap();
                panic!("controller status submission: {error:?}");
            }
        };
        ipc::wait_reply(KERNEL_ASID, call).unwrap();
        let reply = ipc::poll_reply(KERNEL_ASID, call).unwrap().unwrap();
        ipc::close_cap(KERNEL_ASID, call).unwrap();
        ipc::close_cap(KERNEL_ASID, ctl).unwrap();
        if let Some(connection) = reply.cap {
            ipc::close_cap(KERNEL_ASID, connection).unwrap();
        }
        let memory = reply.memory.expect("complete controller deployment record");
        let bytes = crate::memory::object::snapshot_bytes(
            KERNEL_ASID,
            memory,
            usize::try_from(reply.result).expect("controller reply length"),
        );
        crate::memory::object::close_cap(KERNEL_ASID, memory).unwrap();
        let bytes = bytes.unwrap();
        assert_eq!(&bytes[..8], b"CDEPQR01");
        assert_eq!(u64::from_le_bytes(bytes[8..16].try_into().unwrap()), generation);
        let descriptor_len = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        let descriptor = charlotte_launch::deployment::decode(&bytes[20..20 + descriptor_len])
            .expect("current signed controller descriptor");
        assert_eq!(descriptor.artifact_name, b"greet");
        let replicas = &bytes[20 + descriptor_len..];
        assert_eq!(replicas.len(), 12);
        assert_eq!(&replicas[..2], &1u16.to_le_bytes());
        assert_eq!(u64::from_le_bytes(replicas[2..10].try_into().unwrap()), node);
        let variant = u16::from_le_bytes(replicas[10..12].try_into().unwrap());
        assert!(descriptor.variant(usize::from(variant)).is_some());
        logln!(
            "[deploy] controller full-name status generation={generation} node={node:#x} \
             variant={variant}"
        );
    }

    #[cfg(feature = "deploy_net_test")]
    fn fixture_descriptor(node: u64, sequence: u64) -> &'static [u8] {
        let bundle = include_bytes!(env!("CATTEN_LOCAL_DEPLOYMENT_FIXTURE"));
        let mut position = 0;
        while position < bundle.len() {
            let len = u32::from_le_bytes(
                bundle
                    .get(position..position + 4)
                    .expect("fixture descriptor length")
                    .try_into()
                    .unwrap(),
            ) as usize;
            position += 4;
            let bytes = bundle.get(position..position + len).expect("complete fixture descriptor");
            position += len;
            let descriptor =
                charlotte_launch::deployment::decode(bytes).expect("current fixture descriptor");
            if descriptor.node_key == node && descriptor.sequence == sequence {
                assert_eq!(
                    charlotte_launch::deployment::verify(
                        bytes,
                        &charlotte_launch::CLUSTER_PUBLIC_KEY
                    ),
                    charlotte_launch::deployment::VerifyOutcome::Valid
                );
                return bytes;
            }
        }
        panic!("missing signed fixture phase");
    }

    #[cfg(feature = "deploy_net_test")]
    fn run_deploy_phase(
        ns: &NameServiceHandle,
        is_leader: bool,
        dns_conn: u64,
        dns_cfg: *const u8,
    ) {
        use crate::cpu::scheduler::yield_lp;

        // Logical artifact name: "greet".
        // catten_services::dns::artifact_object_id(b"greet").
        const DEPLOY_OBJECT_ID: u64 = 0xfffe_ea29_637f_e28a;
        const DNS_OP_DEPLOY_SUBMIT: u32 = 33;
        const DNS_OP_GET: u32 = 1;
        const AGENT_STAGE_IDENTITY: u32 = 2;
        const AGENT_STAGE_SERVING: u32 = 6;
        const AGENT_STAGE_RETIRED: u32 = 7;

        logln!("[deploy] testing cluster deployment and migration...");

        // The deployed artifact is a note-signed ELF: it must verify against
        // the cluster's public key, and a tampered copy must be refused.
        let artifact = crate::service::store::service_elf(b"greet")
            .expect("[deploy] greet artifact in object store");
        use charlotte_launch::signature_note::VerifyOutcome;
        assert_eq!(
            charlotte_launch::signature_note::verify_elf_for_name(
                artifact,
                &charlotte_launch::CLUSTER_PUBLIC_KEY,
                b"greet",
            ),
            VerifyOutcome::Valid,
            "[deploy] the deployed artifact must carry a valid cluster signature"
        );
        let mut tampered = alloc::vec![0u8; artifact.len()];
        tampered.copy_from_slice(artifact);
        tampered[artifact.len() / 2] ^= 0x01;
        assert_eq!(
            charlotte_launch::signature_note::verify_elf_for_name(
                &tampered,
                &charlotte_launch::CLUSTER_PUBLIC_KEY,
                b"greet",
            ),
            VerifyOutcome::Invalid,
            "[deploy] a tampered artifact must fail signature verification"
        );
        logln!("[deploy] artifact is a note-signed ELF; tampering is refused.");

        // Observe the normal node deployment agent rather than creating a
        // test-only authority holder.
        let agent = crate::service::launch::steady_state()
            .deployment
            .expect("[deploy] operational deployment plane missing")
            .agent;
        logln!("[deploy] observing deployment agent (asid={})", agent.asid);
        let agent_cfg: *const u8 = {
            let base: *mut u8 = agent.status_frame.into();
            base
        };
        let deadline = crate::self_test::results::Deadline::after_millis(120_000);
        while status_word(agent_cfg, charlotte_launch::agent_status::STAGE) < AGENT_STAGE_IDENTITY {
            deadline.assert_pending("EL0 deploy agent identity");
            yield_lp();
        }
        let acknowledged_retirements_before =
            crate::service::supervisor::DEPLOYMENT_ACKNOWLEDGED_RETIREMENTS
                .load(core::sync::atomic::Ordering::Relaxed);
        let forced_retirements_before = crate::service::supervisor::DEPLOYMENT_FORCED_RETIREMENTS
            .load(core::sync::atomic::Ordering::Relaxed);
        let my_key = status_word(agent_cfg, charlotte_launch::agent_status::NODE_KEY) as u64;
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

        // "Software lives in the object store": stage the signed artifact
        // into the (node-local, for now) store before the cluster deploys it.
        // A real cluster would fetch it from a cluster-wide store instead;
        // the per-node staging is one of the documented simulations.
        if stage_deploy_artifact(ns, DEPLOY_OBJECT_ID, artifact).is_err() {
            panic!("[deploy] failed to stage the deploy artifact into the object store");
        }
        logln!("[deploy] staged the note-signed artifact into the object store.");

        // The leader deploys the artifact to the peer node (cross-node
        // hosting is the point of the cluster).
        if is_leader {
            let descriptor = fixture_descriptor(peer_key, 1);
            let request = descriptor;
            logln!(
                "[deploy] calling OP_DEPLOY on the peer (object {:#x} node {peer_key:#x})",
                DEPLOY_OBJECT_ID
            );
            let deadline = crate::self_test::results::Deadline::after_millis(120_000);
            let deploy = loop {
                let result =
                    call_with_memory(dns_conn, DNS_OP_DEPLOY_SUBMIT, request.len() as u64, request);
                if result != Some(-23) {
                    break result;
                }
                // Profile propagation has no fixture subscription ABI. Bounded
                // backoff repeats only explicit pre-commit incompatibility.
                deadline.assert_pending("deployment CPU profile propagation");
                crate::cpu::scheduler::sleep_millis(500);
            };
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
                let stage = status_word(agent_cfg, charlotte_launch::agent_status::STAGE);
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

        let initial_host = if is_leader {
            peer_key
        } else {
            my_key
        };
        let migrated_host = if is_leader {
            my_key
        } else {
            peer_key
        };
        wait_rollout_ready(dns_conn, 1, initial_host);
        if !is_leader {
            // Submit the same signed decision through this follower's local DNS.
            // The current descriptor-only relay must preserve exact idempotence
            // before the public echo barrier permits the next generation.
            let descriptor = fixture_descriptor(my_key, 1);
            assert_eq!(
                call_with_memory(
                    dns_conn,
                    DNS_OP_DEPLOY_SUBMIT,
                    descriptor.len() as u64,
                    descriptor
                ),
                Some(1)
            );
            logln!("[deploy] descriptor-only follower relay preserved generation 1");
        }
        assert_eq!(
            invoke_dns(dns_conn, b"greet", DNS_OP_GET, 0),
            Some(DNS_ERR_NOT_FOUND),
            "legacy DNS proxy must not resolve policy-published greet"
        );
        logln!("[deploy] generation 1 ready on peer; protected service rejects legacy forwarding.");
        // Republish the follower's still-live public echo as a fixture marker.
        // This prevents the leader from advancing revision
        // before the follower has observed exact generation-1 readiness.
        const PHASE_ONE_READY: u64 = ECHO_NAME; // follower's still-live public echo
        let phase_deadline = crate::self_test::results::Deadline::after_millis(120_000);
        if !is_leader {
            loop {
                if call(dns_conn, DNS_OP_REGISTER, PHASE_ONE_READY)
                    .is_some_and(|result| result >= 1)
                {
                    break;
                }
                phase_deadline.assert_pending("deployment follower phase-1 observation");
                crate::cpu::scheduler::sleep_millis(250);
            }
        } else {
            while call(dns_conn, DNS_OP_LOOKUP, PHASE_ONE_READY) != Some(DNS_RESULT_REMOTE) {
                phase_deadline.assert_pending("deployment phase-1 catalog barrier");
                crate::cpu::scheduler::sleep_millis(250);
            }
        }

        // The leader re-deploys the artifact to its own node: the service
        // migrates. The new host registers a fresh generation; the old host
        // retires and its generation-fenced unregister cannot clobber it.
        // Snapshot before publishing the migration. After receiving its
        // post-migration reply, the follower issues a second idempotent call.
        // Observing that causally later request is stronger than watching a
        // transport ACK: relmsg may exhaust the bounded retry lease for an
        // ACK that was lost after the application reply was already
        // delivered.
        let remote_calls_before_migration = is_leader
            .then(|| status_word(dns_cfg, charlotte_launch::dns_status::REMOTE_CALLS_SERVED));
        if is_leader {
            let descriptor = fixture_descriptor(my_key, 2);
            let request = descriptor;
            let deploy =
                call_with_memory(dns_conn, DNS_OP_DEPLOY_SUBMIT, request.len() as u64, request);
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
        while status_word(agent_cfg, charlotte_launch::agent_status::STAGE) != expected_stage {
            deadline.assert_pending("EL0 deploy migration handover");
            crate::cpu::scheduler::sleep_millis(10);
            yield_lp();
        }
        logln!(
            "[deploy] local agent stage {} after migration.",
            status_word(agent_cfg, charlotte_launch::agent_status::STAGE)
        );
        if !is_leader {
            assert!(
                crate::service::supervisor::DEPLOYMENT_ACKNOWLEDGED_RETIREMENTS
                    .load(core::sync::atomic::Ordering::Relaxed)
                    > acknowledged_retirements_before,
                "[deploy] lifecycle-aware greet must acknowledge cooperative retirement"
            );
            assert_eq!(
                crate::service::supervisor::DEPLOYMENT_FORCED_RETIREMENTS
                    .load(core::sync::atomic::Ordering::Relaxed),
                forced_retirements_before,
                "[deploy] cooperative greet retirement must finish before forced termination"
            );
        }

        wait_rollout_ready(dns_conn, 2, migrated_host);
        let post_migration = invoke_dns(dns_conn, b"greet", DNS_OP_GET, 0);
        assert_eq!(
            post_migration,
            Some(DNS_ERR_NOT_FOUND),
            "replacement must preserve protected namespace"
        );
        logln!("[deploy] generation 2 ready; protected service rejects legacy forwarding.");

        // Causal barrier: only the follower can issue this second call, and
        // only after its first post-migration call returned. The leader waits
        // until it has served both requests made after the snapshot. This
        // proves application progress without depending on an internal
        // transport ACK remaining observable after its retry lease ends.
        if !is_leader {
            let barrier = invoke_dns(dns_conn, b"greet", DNS_OP_GET, 0);
            assert_eq!(
                barrier,
                Some(DNS_ERR_NOT_FOUND),
                "[deploy] causal barrier must reach the migrated host and preserve policy denial"
            );
            logln!("[deploy] follower completed the causal post-migration barrier.");
        } else {
            let barrier_deadline = crate::self_test::results::Deadline::after_millis(120_000);
            let calls_before = remote_calls_before_migration.expect("leader remote-call snapshot");
            let required_calls = calls_before.saturating_add(2);
            while status_word(dns_cfg, charlotte_launch::dns_status::REMOTE_CALLS_SERVED)
                < required_calls
            {
                barrier_deadline.assert_pending("EL0 deploy follower causal barrier");
                crate::cpu::scheduler::sleep_millis(10);
                yield_lp();
            }
            logln!("[deploy] leader observed the follower's causal post-migration barrier.");
        }

        // Exercise the other terminal path as well. The leader currently
        // hosts greet. Test-only kernel code shortens that already-admitted
        // domain's grace period to zero, then moves the desired assignment
        // back to the follower. Descriptor decoding independently tests the
        // signed zero-grace value; this phase proves the runtime force path,
        // reaping, and generation-fenced replacement end to end.
        let forced_before_return_migration =
            crate::service::supervisor::DEPLOYMENT_FORCED_RETIREMENTS
                .load(core::sync::atomic::Ordering::Relaxed);
        if is_leader {
            let principal = charlotte_launch::artifact_principal_id(b"greet");
            let mut deployed = crate::service::supervisor::DEPLOYED_DOMAINS.lock();
            let running = deployed
                .iter_mut()
                .find(|entry| entry.principal == principal)
                .expect("[deploy] leader-owned greet domain before forced migration");
            running.shutdown_grace_ms = 0;
            drop(deployed);

            let descriptor = fixture_descriptor(peer_key, 3);
            let request = descriptor;
            let deploy =
                call_with_memory(dns_conn, DNS_OP_DEPLOY_SUBMIT, request.len() as u64, request);
            logln!("[deploy] zero-grace return migration result = {deploy:?}");
            assert!(
                deploy.is_some_and(|generation| generation >= 3),
                "[deploy] return migration must commit a third generation"
            );
        }

        let deadline = crate::self_test::results::Deadline::after_millis(120_000);
        if is_leader {
            // The agent only reports RETIRED once *every* deployment retires,
            // and unrelated deployments may exist on the host. Wait for this artifact's own domain
            // to be reaped instead of the agent's global stage.
            let principal = charlotte_launch::artifact_principal_id(b"greet");
            while crate::service::supervisor::DEPLOYED_DOMAINS
                .lock()
                .iter()
                .any(|entry| entry.principal == principal)
            {
                deadline.assert_pending("EL0 deploy forced return migration");
                crate::cpu::scheduler::sleep_millis(10);
                yield_lp();
            }
            assert!(
                crate::service::supervisor::DEPLOYMENT_FORCED_RETIREMENTS
                    .load(core::sync::atomic::Ordering::Relaxed)
                    > forced_before_return_migration,
                "[deploy] zero-grace retirement must take the forced path"
            );
        } else {
            while status_word(agent_cfg, charlotte_launch::agent_status::STAGE)
                != AGENT_STAGE_SERVING
            {
                deadline.assert_pending("EL0 deploy forced return migration");
                crate::cpu::scheduler::sleep_millis(10);
                yield_lp();
            }
        }

        wait_rollout_ready(dns_conn, 3, initial_host);
        assert_eq!(invoke_dns(dns_conn, b"greet", DNS_OP_GET, 0), Some(DNS_ERR_NOT_FOUND));
        logln!(
            "[deploy] forced retirement preserved generation-3 readiness and protected namespace."
        );

        logln!(
            "[deploy] SUCCESS: signed architecture variants reached readiness across three pinned \
             generations, with cooperative and zero-grace forced retirement; legacy forwarding \
             remained denied."
        );
    }

    extern "C" fn verify_el0_dns() {
        use crate::cpu::scheduler::yield_lp;

        let ns = unsafe { TEST_STATE.as_ref() }.expect("[dns] test state missing");
        let kernel_ns = kernel_ns_connection(ns);

        let dns = crate::service::launch::steady_state()
            .cluster
            .expect("[dns] operational cluster services missing")
            .dns;
        logln!("[dns] observing operational replica (asid={})", dns.asid);

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

        // Serving begins from the durable singleton configuration. Wait for
        // discovery-driven admission to commit and stabilize before testing
        // catalog replication; a singleton registration would not prove the
        // networked cluster path.
        let membership_deadline = crate::self_test::results::Deadline::after_millis(120_000);
        let mut peers = status_word(dns_cfg, charlotte_launch::dns_status::PEER_COUNT);
        while peers < 2 {
            membership_deadline.assert_pending("EL0 DNS cluster membership");
            yield_lp();
            peers = status_word(dns_cfg, charlotte_launch::dns_status::PEER_COUNT);
        }
        logln!("[dns] Raft membership peers = {peers}");

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

        // Catalog-only fixture markers use the explicit event operation;
        // service registration requires a local publication or attachment.
        if state == 3 {
            logln!("[dns] replica is the Raft leader; registering name.");
            let register = call_with_memory(dns_conn, DNS_OP_EVENT_FIRE, 5, b"alpha");
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
        // node; either proves the catalog entry replicated through the
        // cluster. The leader publishes the name through a replicated
        // commit, so a follower may observe its converged catalog slightly
        // before the leader's own registration is authoritative; retry the
        // lookup briefly instead of asserting on the first attempt.
        let lookup_deadline = crate::self_test::results::Deadline::after_millis(30_000);
        let lookup = loop {
            let lookup = call(dns_conn, DNS_OP_LOOKUP, ALPHA_NAME);
            if lookup == Some(DNS_RESULT_LOCAL) || lookup == Some(DNS_RESULT_REMOTE) {
                break lookup;
            }
            lookup_deadline.assert_pending("EL0 dns replicated lookup");
            yield_lp();
        };
        logln!("[dns] lookup result = {lookup:?}");
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
        let is_leader = state == 3;
        let manifest = [crate::service::bootstrap::ManifestEntry {
            key: charlotte_launch::echo_config::CLUSTER_NAME,
            flags: 0,
            value: crate::service::bootstrap::ManifestValue::Bytes(LONG_ECHO_NAME),
        }];
        let echo = crate::service::supervisor::spawn_with_manifest(
            crate::service::store::service_elf(b"echo").expect("[el0_dns] echo.elf"),
            ns,
            ConnectionRights::CALL,
            if is_leader {
                &manifest
            } else {
                &[]
            },
        );
        logln!("[dns] echo spawned (asid={})", echo.asid);

        let mut echo_generation = 0;
        if is_leader {
            // A prior lifecycle test may have left a closed echo generation
            // in the local registry. The new domain's own serving stage is
            // the authoritative proof that it has replaced that entry; a
            // mere successful lookup could still return the stale generation.
            let deadline = crate::self_test::results::Deadline::after_millis(30_000);
            let echo_status: *const u8 = {
                let base: *mut u8 = echo.status_frame.into();
                base
            };
            while unsafe {
                crate::self_test::status_u32(echo_status, charlotte_launch::echo_status::STAGE)
            } < 6
            {
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
            let local_generation = generation_request(
                kernel_ns,
                LONG_ECHO_NAME,
                0,
                charlotte_protocol_ns::Action::Observe,
            )
            .expect("long publication observation");
            assert!(local_generation >= 1);
            assert_eq!(
                call(kernel_ns, charlotte_protocol_ns::OP_PUBLICATION_GENERATION, ECHO_NAME),
                Some(-2),
                "retired scalar observation must reject"
            );
            assert_eq!(
                call_with_memory(
                    kernel_ns,
                    charlotte_protocol_ns::OP_UNREGISTER_GENERATION,
                    ECHO_NAME,
                    &(local_generation as u64).to_le_bytes()
                ),
                Some(-2),
                "retired packed-generation retirement must reject"
            );
            assert_eq!(
                generation_request(
                    kernel_ns,
                    LONG_ECHO_NAME,
                    local_generation as u64 + 1,
                    charlotte_protocol_ns::Action::Retire
                ),
                Some(-1),
                "stale long-name retirement must leave publication intact"
            );
            assert_eq!(
                generation_request(
                    kernel_ns,
                    LONG_ECHO_NAME,
                    0,
                    charlotte_protocol_ns::Action::Observe
                ),
                Some(local_generation)
            );
            logln!(
                "[dns] full-name publication observed; retired and stale generation requests \
                 rejected"
            );
        }

        // Wait for the catalog to carry both names.
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        loop {
            catalog = status_word(dns_cfg, charlotte_launch::dns_status::CATALOG_ENTRIES);
            if catalog >= 3 {
                logln!("[dns] catalog carries {catalog} name(s); invoking echo.");
                break;
            }
            deadline.assert_pending("EL0 dns echo publication");
            yield_lp();
        }

        // Invoke the echo service by name. If this node hosts it the dns calls
        // it locally; otherwise the dns relays the call to the hosting node.
        // Retired and malformed invocation envelopes must reject without
        // dispatching to the live echo service.
        let mut retired = Vec::with_capacity(12);
        retired.extend_from_slice(&ECHO_OP_ECHO.to_le_bytes());
        retired.extend_from_slice(&42i64.to_le_bytes());
        assert_eq!(
            call_with_memory(dns_conn, DNS_OP_CALL, ECHO_NAME, &retired),
            Some(DNS_ERR_TOO_LARGE)
        );
        let mut trailing = charlotte_protocol_dns::encode(charlotte_protocol_dns::Request {
            name: b"echo",
            opcode: ECHO_OP_ECHO,
            arg: 42,
        })
        .expect("DNS rejection fixture encoding");
        trailing.push(0);
        assert_eq!(
            call_with_memory(dns_conn, DNS_OP_CALL, trailing.len() as u64, &trailing),
            Some(DNS_ERR_BAD_OPCODE)
        );
        logln!("[dns] retired and trailing invocation envelopes rejected");
        // The catalog entry can reach a follower before that replica has
        // learned the current leader (and the two QEMU guests need not boot at
        // the same rate).  Echo is idempotent, so transient routing/election
        // errors are safely retryable until both applied views converge.
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        let echo_result = loop {
            let result = invoke_dns(dns_conn, b"echo", ECHO_OP_ECHO, 42);
            if result == Some(42) {
                break result;
            }
            deadline.assert_pending("EL0 dns remote echo leader convergence");
            crate::cpu::scheduler::sleep_millis(250);
            yield_lp();
        };
        logln!("[dns] remote echo result = {echo_result:?}");
        assert_eq!(
            echo_result,
            Some(42),
            "[dns] cross-node invocation of echo must return the echoed value"
        );

        let long_deadline = crate::self_test::results::Deadline::after_millis(60_000);
        loop {
            let result = invoke_dns(dns_conn, LONG_ECHO_NAME, ECHO_OP_ECHO, 43);
            if result == Some(43) {
                break;
            }
            long_deadline.assert_pending("EL0 dns full-name echo convergence");
            crate::cpu::scheduler::sleep_millis(200);
        }
        logln!("[dns] full-name echo result = 43");

        // The leader's own invocation is local. Keep the leader VM alive
        // until its DNS replica has also served the follower's remote call;
        // otherwise the runner can observe local SELFTEST COMPLETE and stop
        // the leader while the follower is still awaiting its reply.
        let mut query_barrier = 0;
        if is_leader {
            let deadline = crate::self_test::results::Deadline::after_millis(60_000);
            while status_word(dns_cfg, charlotte_launch::dns_status::REMOTE_CALL_ACKS) < 2 {
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
            assert_eq!(
                status_word(dns_cfg, charlotte_launch::dns_status::CATALOG_ENTRIES),
                4,
                "[dns] catalog must contain alpha, both echo names, and the membership event"
            );
            crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
                .read()
                .abort_thread_generation(echo.tid, echo.generation)
                .expect("[dns] hosted echo abort");
            crate::service::supervisor::wait_domain_exit(&echo, 30_000);
            crate::service::supervisor::teardown_domain(echo).expect("[dns] echo reclamation");
            logln!("[dns] generation {echo_generation} endpoint closed; awaiting tombstone.");
        }

        // Keep both voters alive until the exact-generation tombstone has
        // replicated. This also prevents the follower's runner from removing
        // quorum while the leader is committing the unregister operation.
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        let mut unregister_spins = 0u64;
        // The membership event is cluster history and remains after the
        // hosted echo service is removed, alongside alpha.
        while status_word(dns_cfg, charlotte_launch::dns_status::CATALOG_ENTRIES) != 2 {
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
            // This replica's `registered_count` passes through 2 twice: once
            // while it is still catching up (alpha + echo) and again after the
            // tombstone commits (alpha + the membership event). Waiting on the
            // count alone would let the follower leave this phase before the
            // unregister has replicated, so wait on the observable outcome:
            // the name must stop resolving.
            let tombstone_deadline = crate::self_test::results::Deadline::after_millis(60_000);
            let mut lookup = call(dns_conn, DNS_OP_LOOKUP, ECHO_NAME);
            while lookup != Some(DNS_ERR_NOT_FOUND) {
                tombstone_deadline.assert_pending("EL0 dns follower tombstone observation");
                crate::cpu::scheduler::sleep_millis(200);
                lookup = call(dns_conn, DNS_OP_LOOKUP, ECHO_NAME);
            }
        }
        if is_leader {
            // The ordinary named lookup distinguishes an unpublished tombstone
            // from a retained connection to a dead endpoint. Observe read-only
            // status under a bounded fixture budget; never submit a removal to
            // make the implementation's cleanup appear complete.
            let cleanup_deadline = crate::self_test::results::Deadline::after_millis(30_000);
            loop {
                let result =
                    call_with_memory(kernel_ns, 16, LONG_ECHO_NAME.len() as u64, LONG_ECHO_NAME);
                if result == Some(-1) {
                    break;
                }
                cleanup_deadline.assert_pending("EL0 dns full-name local retirement completion");
                crate::cpu::scheduler::sleep_millis(200);
            }
            logln!("[dns] full-name local generation retired after endpoint tombstone");
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
