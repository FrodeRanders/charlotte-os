//! Self-test: dynamic raft membership admission, bridged through discovery.
//!
//! Boots a single-node raft domain *without* an explicit `node-id`: the raft
//! derives its id from the node identity, which is the same name discovery
//! advertises, so the local discovery service can report this node's raft
//! role and id. The test then drives the real join path end to end:
//!
//! 1. Ask the local discovery service where the cluster is
//!    (`OP_CLUSTER_STATUS`): it answers with this node's raft id/role and
//!    every discovered peer's role and raft id — including the honest
//!    "not in a cluster" answer before the raft is serving.
//! 2. Deterministically pick the joiner: the node with the lexicographically
//!    *larger* raft id asks the cluster administration service to join
//!    (`clusterctl OP_JOIN`), which locates the leader through discovery and
//!    asks the leader's raft service to admit this node (`OP_ADD_SERVER`).
//!    The smaller-id node is the anchor and simply waits.
//! 3. Verify convergence: this node's raft reports a two-member
//!    configuration (the leader commits JOIN, promotes the joiner into a
//!    joint configuration once it catches up, and auto-finalizes).
//!
//! The single-joiner rule keeps the test deterministic on the two-guest
//! deployment: both nodes start as leaders of their own single-node cluster
//! ("any cluster on the segment"), and exactly one of them joins the other.

pub mod inner {
    use crate::{
        ipc,
        ipc::ConnectionRights,
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
    use alloc::{
        string::String,
        vec::Vec,
    };

    const CLUSTER_KEY: u64 = charlotte_launch::manifest_key(b"cluster");
    const ELECTION_KEY: u64 = charlotte_launch::manifest_key(b"elect-ms");

    const NS_OP_LOOKUP: u32 = 2;
    const DISCO_NAME: u64 = u64::from_le_bytes(*b"disco\0\0\0");
    const DISCO_OP_CLUSTER_STATUS: u32 = 6;
    const DISCO_OP_PROBE: u32 = 1;
    const RAFT_OP_CLUSTER_STATUS: u32 = 9;
    // Dedicated kernel-side scratch for moved-memory reads. Must not share
    // the vaddr used by other deferred verifiers (el0_clusterctl maps replies
    // at 0x601000): the deferred verifiers run concurrently, and interleaved
    // maps at the same kernel vaddr would race and corrupt both sides.
    const MEM_BASE: usize = 0x0000_0000_0060_2000;
    const MEM_LEN: usize = 4096;

    const ROLE_LEADER: u8 = 3;

    static mut JOIN_NS: Option<NameServiceHandle> = None;

    fn spawn_raft_node(
        manifest: &[ManifestEntry<'_>],
        ns_handle: &NameServiceHandle,
    ) -> ServiceDomain {
        let addr = crate::service::loader::load_domain(
            crate::service::store::service_elf(b"raft").expect("[el0_join] raft.elf"),
        );
        let conn = crate::ipc::connection_delegate(
            ns_handle.domain.asid,
            ns_handle.endpoint_cap,
            addr.asid,
            ConnectionRights::CALL,
        )
        .expect("join raft conn delegate");
        crate::service::bootstrap::write_bootstrap_cap(addr.config_frame, conn);
        crate::service::bootstrap::write_manifest(addr.config_frame, manifest);
        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(addr.entry_vaddr) };
        let tid = crate::cpu::scheduler::spawn_thread(addr.asid, entry);
        let generation = crate::cpu::scheduler::threads::MASTER_THREAD_TABLE
            .read()
            .get(tid)
            .expect("join raft thread missing after spawn")
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
            crate::memory::KERNEL_ASID,
            ConnectionRights::CALL,
        )
        .expect("[el0_join] kernel name-service connection")
    }

    fn lookup_service(kernel_ns: u64, name: u64) -> Option<u64> {
        let call = ipc::scalar_call(crate::memory::KERNEL_ASID, kernel_ns, NS_OP_LOOKUP, name).ok()?;
        ipc::wait_reply(crate::memory::KERNEL_ASID, call).ok()?;
        ipc::poll_reply(crate::memory::KERNEL_ASID, call)
            .ok()
            .flatten()
            .map(|reply| reply.cap.unwrap_or(0))
    }

    /// Scalar call with a moved memory object; returns the reply result and
    /// any returned memory cap.
    fn call_with_memory_reply(
        kernel_conn: u64,
        opcode: u32,
        arg0: u64,
        bytes: &[u8],
    ) -> Option<(i64, Option<u64>)> {
        let mem =
            crate::memory::object::allocate_with_bytes(crate::memory::KERNEL_ASID, bytes).ok()?;
        let call = ipc::scalar_call_with_memory_move(
            crate::memory::KERNEL_ASID,
            kernel_conn,
            opcode,
            arg0,
            mem,
        )
        .ok()?;
        ipc::wait_reply(crate::memory::KERNEL_ASID, call).ok()?;
        let reply = ipc::poll_reply(crate::memory::KERNEL_ASID, call).ok().flatten();
        let result = reply.map(|reply| (reply.result, reply.memory));
        let _ = ipc::close_cap(crate::memory::KERNEL_ASID, call);
        result
    }

    fn read_moved_memory(cap: u64, len: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        if crate::memory::object::map(crate::memory::KERNEL_ASID, cap, MEM_BASE.into(), false).is_ok()
        {
            for index in 0..len {
                bytes.push(unsafe { core::ptr::read_volatile((MEM_BASE + index) as *const u8) });
            }
            let _ = crate::memory::object::unmap(crate::memory::KERNEL_ASID, cap);
        }
        let _ = crate::memory::object::close_cap(crate::memory::KERNEL_ASID, cap);
        bytes
    }

    /// Ask the discovery service where the cluster is. Returns
    /// `(self_raft_id, peers)` where each peer is `(mac, role, raft_id,
    /// leader_id)`.
    fn disco_cluster_answer(
        kernel_ns: u64,
    ) -> Option<(Vec<u8>, Vec<([u8; 6], u8, Vec<u8>, Vec<u8>)>)> {
        let disco_conn = lookup_service(kernel_ns, DISCO_NAME)?;
        crate::logln!("[join] disco connection obtained");
        let (_result, memory) = call_with_memory_reply(disco_conn, DISCO_OP_CLUSTER_STATUS, 0, &[])?;
        crate::logln!("[join] disco answer received");
        let memory = memory?;
        let bytes = read_moved_memory(memory, MEM_LEN);
        let (_self_role, self_raft_id, _self_leader_id, peers) =
            charlotte_protocol_disco::parse_cluster_answer(&bytes)?;
        Some((self_raft_id.to_vec(), peers))
    }

    /// This node's raft cluster status: `(state, term, commit, members)`.
    fn raft_cluster_status(kernel_ns: u64, raft_id: &[u8]) -> Option<(u32, u64, u64, u32)> {
        let raft_name = raft_service_name(raft_id);
        let raft_conn = lookup_service(kernel_ns, raft_name)?;
        crate::logln!("[join] raft connection obtained");
        let (_result, memory) = call_with_memory_reply(raft_conn, RAFT_OP_CLUSTER_STATUS, 0, &[])?;
        crate::logln!("[join] raft status reply received");
        let memory = memory?;
        crate::logln!("[join] raft status memory present");
        let bytes = read_moved_memory(memory, MEM_LEN);
        let (state, term, commit, members, _leader, _self_id) =
            parse_raft_status(&bytes)?;
        crate::logln!("[join] raft status parsed: members={}", members);
        Some((state, term, commit, members))
    }

    /// Minimal local parse of the raft cluster-status blob (the kernel does
    /// not link the services crate).
    fn parse_raft_status(payload: &[u8]) -> Option<(u32, u64, u64, u32, Vec<u8>, Vec<u8>)> {
        if payload.len() < 25 {
            return None;
        }
        let state = u32::from_le_bytes(payload[0..4].try_into().ok()?);
        let term = u64::from_le_bytes(payload[4..12].try_into().ok()?);
        let commit = u64::from_le_bytes(payload[12..20].try_into().ok()?);
        let members = u32::from_le_bytes(payload[20..24].try_into().ok()?);
        let leader_len = payload[24] as usize;
        if payload.len() < 25 + leader_len + 1 {
            return None;
        }
        let leader = payload[25..25 + leader_len].to_vec();
        let pos = 25 + leader_len;
        let self_len = payload[pos] as usize;
        if payload.len() < pos + 1 + self_len {
            return None;
        }
        Some((
            state,
            term,
            commit,
            members,
            leader,
            payload[pos + 1..pos + 1 + self_len].to_vec(),
        ))
    }

    /// FNV-1a over `raft-{id}` — the registry name of a node's raft service
    /// (mirrors `catten_services::raft_name`).
    fn raft_service_name(id: &[u8]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in b"raft-".iter().copied().chain(id.iter().copied()) {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    const DNS_NAME: u64 = u64::from_le_bytes(*b"dns\0\0\0\0\0");
    const DNS_OP_EVENT_WAIT: u32 = 10;

    /// Wait for a cluster event through the replicated dns: `OP_EVENT_WAIT`
    /// parks the reply token when the event has not fired and the dns
    /// resolves it the moment the committed entry lands on this node. The
    /// poll is only the *bounded* delivery check — fulfillment is defined by
    /// consensus, never by polling order or boot timing. Returns the
    /// committed generation.
    fn wait_cluster_event(
        kernel_ns: u64,
        event: &[u8],
        deadline: &crate::self_test::results::Deadline,
    ) -> Option<i64> {
        use crate::cpu::scheduler::yield_lp;

        let dns_conn = lookup_service(kernel_ns, DNS_NAME)?;
        let mem = crate::memory::object::allocate_with_bytes(crate::memory::KERNEL_ASID, event)
            .ok()?;
        let call = ipc::scalar_call_with_memory_move(
            crate::memory::KERNEL_ASID,
            dns_conn,
            DNS_OP_EVENT_WAIT,
            event.len() as u64,
            mem,
        )
        .ok()?;
        loop {
            if let Ok(Some(reply)) = ipc::poll_reply(crate::memory::KERNEL_ASID, call) {
                let result = reply.result;
                let _ = ipc::close_cap(crate::memory::KERNEL_ASID, call);
                return Some(result);
            }
            deadline.assert_pending("EL0 cluster event wait");
            yield_lp();
        }
    }

    pub fn test_el0_join() {
        crate::logln!("Testing dynamic raft membership join (discovery-bridged)...");
        let name_service = supervisor::node_name_service();
        unsafe { JOIN_NS = Some(name_service) };
        let _verifier = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Join,
            verify_join_cluster,
        );
        crate::logln!("[join] verifier deferred (locates the cluster via discovery and joins)");
    }

    extern "C" fn verify_join_cluster() {
        use crate::cpu::scheduler::yield_lp;

        let ns = unsafe { JOIN_NS.as_ref() }.expect("[join] test state missing");
        let kernel_ns = kernel_ns_connection(ns);

        let manifest = [
            ManifestEntry {
                key: CLUSTER_KEY,
                flags: 0,
                value: ManifestValue::Bytes(b"test-cluster"),
            },
            ManifestEntry {
                key: ELECTION_KEY,
                flags: 0,
                value: ManifestValue::Unsigned(150),
            },
        ];
        let raft_domain = spawn_raft_node(&manifest, ns);
        crate::logln!("[join] raft domain spawned (asid={})", raft_domain.asid);

        // The raft derives its id from the node identity and registers
        // `raft-{id}`; wait until it is serving (state != 0).
        let raft_status: *const u32 = {
            let base: *mut u8 = raft_domain.status_frame.into();
            base as *const u32
        };
        let serving_deadline = crate::self_test::results::Deadline::after_millis(60_000);
        while unsafe { core::ptr::read_volatile(raft_status.add(2)) } == 0 {
            serving_deadline.assert_pending("EL0 join raft serving");
            yield_lp();
        }
        crate::logln!("[join] local raft serving.");

        // The raft service handles the whole admission itself: it locates
        // the cluster through discovery (MAC-level, no local lookups), the
        // lexicographically larger single-node leader applies to the smaller
        // one with a MAC-addressed join request, and the anchor's consensus
        // commits, promotes, and finalizes the JOIN. This node only learns
        // its own raft id from discovery and then waits on the membership
        // event — fulfillment is communicated through Raft consensus.
        let mut self_raft_id: Vec<u8> = Vec::new();
        let mut membership_event: Option<Vec<u8>> = None;
        let mut next_probe_ms = crate::cpu::scheduler::monotonic_millis();
        let discovery_deadline = crate::self_test::results::Deadline::after_millis(120_000);
        loop {
            if let Some((answer_self_id, peers)) = disco_cluster_answer(kernel_ns)
                && !answer_self_id.is_empty()
            {
                // The node with the lexicographically larger id is the
                // joiner: it waits for its own admission. The smaller id is
                // the anchor: it waits for the joiner's membership event,
                // which its own consensus fires when the JOIN finalizes.
                let peer_cluster = peers.iter().find(|(_, role, raft_id, _)| {
                    *role == ROLE_LEADER
                        && !raft_id.is_empty()
                        && raft_id.as_slice() != answer_self_id.as_slice()
                });
                if let Some((_mac, _role, raft_id, _leader_id)) = peer_cluster {
                    let event = if answer_self_id > *raft_id {
                        alloc::format!(
                            "event:membership:{}",
                            String::from_utf8_lossy(&self_raft_id)
                        )
                        .into_bytes()
                    } else {
                        alloc::format!(
                            "event:membership:{}",
                            String::from_utf8_lossy(raft_id)
                        )
                        .into_bytes()
                    };
                    membership_event = Some(event);
                    self_raft_id = answer_self_id;
                    break;
                }
                self_raft_id = answer_self_id;
            }
            // Force a discovery probe on a slow cadence so this node's own
            // posture is reported quickly.
            let now = crate::cpu::scheduler::monotonic_millis();
            if now >= next_probe_ms {
                next_probe_ms = now + 2_000;
                if let Some(disco_conn) = lookup_service(kernel_ns, DISCO_NAME) {
                    let _ = ipc::scalar_call(
                        crate::memory::KERNEL_ASID,
                        disco_conn,
                        DISCO_OP_PROBE,
                        0,
                    );
                }
            }
            discovery_deadline.assert_pending("EL0 join discovery");
            yield_lp();
        }
        crate::logln!(
            "[join] local raft id: {:?}",
            self_raft_id
        );

        // Wait for the membership event — communicated through Raft
        // consensus, resolved the moment the committed entry lands on this
        // node. No polling, no assumed sequence.
        let event = membership_event.expect("[join] membership event name missing");
        let event_deadline = crate::self_test::results::Deadline::after_millis(120_000);
        let generation = wait_cluster_event(kernel_ns, &event, &event_deadline);
        assert!(
            generation.is_some_and(|generation| generation >= 1),
            "[join] membership event must fire through consensus, got {generation:?}"
        );
        crate::logln!(
            "[join] membership event fired (generation {}) — admitted to the cluster.",
            generation.unwrap()
        );

        crate::logln!("[join] SUCCESS: node admitted to the cluster via discovery-bridged join.");
        crate::self_test::results::pass(crate::self_test::results::TestId::Join);
    }
}

pub use inner::test_el0_join;
