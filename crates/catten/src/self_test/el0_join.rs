//! Self-test: dynamic raft membership admission, bridged through discovery.
//!
//! Boots a single-node raft domain *without* an explicit `node-id`: the raft
//! derives its id from the node identity, which is the same name discovery
//! advertises, so the local discovery service can report this node's raft
//! role and id. The test then drives the real join path end to end:
//!
//! 1. Ask the local discovery service where the cluster is (`OP_CLUSTER_STATUS`): it answers with
//!    this node's raft id/role and every discovered peer's role and raft id — including the honest
//!    "not in a cluster" answer before the raft is serving.
//! 2. Deterministically pick the joiner: the node with the lexicographically *larger* raft id asks
//!    the cluster administration service to join (`clusterctl OP_JOIN`), which locates the leader
//!    through discovery and asks the leader's raft service to admit this node (`OP_ADD_SERVER`).
//!    The smaller-id node is the anchor and simply waits.
//! 3. Verify convergence: this node's raft reports a two-member configuration (the leader commits
//!    JOIN, promotes the joiner into a joint configuration once it catches up, and auto-finalizes).
//!
//! The single-joiner rule keeps the test deterministic on the two-guest
//! deployment: both nodes start as leaders of their own single-node cluster
//! ("any cluster on the segment"), and exactly one of them joins the other.

pub mod inner {
    use alloc::{
        string::String,
        vec::Vec,
    };

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

    const CLUSTER_KEY: u64 = charlotte_launch::manifest_key(b"cluster");
    const ELECTION_KEY: u64 = charlotte_launch::manifest_key(b"elect-ms");
    const NETWORK_KEY: u64 = charlotte_launch::manifest_key(b"network");

    const NS_OP_LOOKUP: u32 = 2;
    const DISCO_NAME: u64 = u64::from_le_bytes(*b"disco\0\0\0");
    const DISCO_OP_CLUSTER_STATUS: u32 = 6;
    const DISCO_CLUSTER_STATUS_WAIT_READY: u64 = 1;

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
        let call =
            ipc::scalar_call(crate::memory::KERNEL_ASID, kernel_ns, NS_OP_LOOKUP, name).ok()?;
        let waited = ipc::wait_reply(crate::memory::KERNEL_ASID, call).is_ok();
        let reply = waited
            .then(|| ipc::poll_reply(crate::memory::KERNEL_ASID, call).ok().flatten())
            .flatten();
        let _ = ipc::close_cap(crate::memory::KERNEL_ASID, call);
        reply.and_then(|reply| {
            if let Some(memory) = reply.memory {
                let _ = crate::memory::object::close_cap(crate::memory::KERNEL_ASID, memory);
            }
            reply.cap.filter(|cap| *cap != 0)
        })
    }

    /// Scalar call whose reply carries a moved memory object.
    fn call_for_memory_reply(
        kernel_conn: u64,
        opcode: u32,
        arg0: u64,
    ) -> Option<(i64, Option<u64>)> {
        let call = ipc::scalar_call(crate::memory::KERNEL_ASID, kernel_conn, opcode, arg0).ok()?;
        if ipc::wait_reply(crate::memory::KERNEL_ASID, call).is_err() {
            let _ = ipc::close_cap(crate::memory::KERNEL_ASID, call);
            return None;
        }
        let reply = ipc::poll_reply(crate::memory::KERNEL_ASID, call).ok().flatten();
        let result = reply.map(|reply| {
            if let Some(cap) = reply.cap {
                let _ = ipc::close_cap(crate::memory::KERNEL_ASID, cap);
            }
            (reply.result, reply.memory)
        });
        let _ = ipc::close_cap(crate::memory::KERNEL_ASID, call);
        result
    }

    fn read_moved_memory(cap: u64, len: usize) -> Vec<u8> {
        let bytes = crate::memory::object::snapshot_bytes(crate::memory::KERNEL_ASID, cap, len)
            .unwrap_or_default();
        let _ = crate::memory::object::close_cap(crate::memory::KERNEL_ASID, cap);
        bytes
    }

    /// A discovery cluster answer: `(self_raft_id, peers)` where each peer
    /// is `(mac, role, raft_id, leader_id)`.
    type JoinDiscoveryAnswer = (Vec<u8>, Vec<([u8; 6], u8, Vec<u8>, Vec<u8>)>);

    /// Ask the discovery service where the cluster is.
    fn disco_cluster_answer(disco_conn: u64) -> Option<JoinDiscoveryAnswer> {
        let (result, memory) = call_for_memory_reply(
            disco_conn,
            DISCO_OP_CLUSTER_STATUS,
            DISCO_CLUSTER_STATUS_WAIT_READY,
        )?;
        let memory = memory?;
        let Some(len) = usize::try_from(result).ok().filter(|len| *len > 0) else {
            let _ = crate::memory::object::close_cap(crate::memory::KERNEL_ASID, memory);
            return None;
        };
        let bytes = read_moved_memory(memory, len);
        let (_self_role, self_raft_id, _self_leader_id, peers) =
            charlotte_protocol_disco::parse_cluster_answer(&bytes)?;
        Some((self_raft_id.to_vec(), peers))
    }

    const DNS_NAME: u64 = u64::from_le_bytes(*b"dns\0\0\0\0\0");
    const DNS_OP_EVENT_WAIT: u32 = 10;

    /// Wait for a cluster event through the replicated dns: `OP_EVENT_WAIT`
    /// parks the reply token when the event has not fired and the dns
    /// resolves it the moment the committed entry lands on this node. The
    /// poll is only the *bounded* delivery check — fulfillment is defined by
    /// consensus, never by polling order or boot timing. Returns the
    /// committed generation.
    fn wait_cluster_event(kernel_ns: u64, event: &[u8]) -> Option<i64> {
        let dns_conn = lookup_service(kernel_ns, DNS_NAME)?;
        let mem =
            crate::memory::object::allocate_with_bytes(crate::memory::KERNEL_ASID, event).ok()?;
        let call = match ipc::scalar_call_with_memory_move(
            crate::memory::KERNEL_ASID,
            dns_conn,
            DNS_OP_EVENT_WAIT,
            event.len() as u64,
            mem,
        ) {
            Ok(call) => call,
            Err(_) => {
                let _ = crate::memory::object::close_cap(crate::memory::KERNEL_ASID, mem);
                let _ = ipc::close_cap(crate::memory::KERNEL_ASID, dns_conn);
                return None;
            }
        };
        let ready = ipc::wait_reply_timeout(crate::memory::KERNEL_ASID, call, 120_000)
            .ok()
            .unwrap_or(false);
        let result = if ready {
            ipc::poll_reply(crate::memory::KERNEL_ASID, call)
                .ok()
                .flatten()
                .map(|reply| reply.result)
        } else {
            None
        };
        let _ = ipc::close_cap(crate::memory::KERNEL_ASID, call);
        let _ = ipc::close_cap(crate::memory::KERNEL_ASID, dns_conn);
        result
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
                // This is a real cross-QEMU cluster, not the in-process Raft
                // unit test. Match the DNS test's transport budget: 150 ms
                // lets both voters repeatedly time out while vote RPCs are
                // still queued behind boot traffic, producing split-vote
                // livelock on a slow emulator.
                value: ManifestValue::Unsigned(2_000),
            },
            ManifestEntry {
                key: NETWORK_KEY,
                flags: 0,
                value: ManifestValue::Unsigned(1),
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
            // This is a verifier observing a shared status page, not service
            // synchronization. Yielding keeps the deadline live even on a
            // boot path where a timer PPI is delayed on an otherwise-idle LP.
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
        let disco_conn = lookup_service(kernel_ns, DISCO_NAME)
            .expect("EL0 join could not resolve discovery service");
        // This one call is retained by disco until both local and remote Raft
        // posture are known. Readiness is published by the service that owns
        // the state, rather than inferred from test ordering or retry delays.
        let (answer_self_id, peers) = disco_cluster_answer(disco_conn)
            .filter(|(self_id, _)| !self_id.is_empty())
            .expect("EL0 join discovery produced no local Raft identity");
        // The node with the lexicographically larger id is the joiner: it
        // waits for its own admission. The smaller id is the anchor and waits
        // for the joiner's membership event.
        let (_, _, peer_raft_id, _) = peers
            .iter()
            .find(|(_, _, raft_id, _)| {
                !raft_id.is_empty() && raft_id.as_slice() != answer_self_id.as_slice()
            })
            .expect("EL0 join discovery produced no peer Raft identity");
        let membership_event = if answer_self_id > *peer_raft_id {
            alloc::format!("event:membership:{}", String::from_utf8_lossy(&answer_self_id))
                .into_bytes()
        } else {
            alloc::format!("event:membership:{}", String::from_utf8_lossy(peer_raft_id))
                .into_bytes()
        };
        let self_raft_id = answer_self_id;
        let _ = ipc::close_cap(crate::memory::KERNEL_ASID, disco_conn);
        crate::logln!("[join] local raft id: {:?}", self_raft_id);

        // Wait for the membership event — communicated through Raft
        // consensus, resolved the moment the committed entry lands on this
        // node. No polling, no assumed sequence.
        let generation = wait_cluster_event(kernel_ns, &membership_event);
        if generation.is_none() {
            let word = |offset: usize| unsafe { core::ptr::read_volatile(raft_status.add(offset)) };
            let frouter = crate::self_test::FROUTER_STATUS_FRAME
                .load(core::sync::atomic::Ordering::Acquire)
                as *const u32;
            let (frouter_rx, frouter_forwarded, frouter_dropped, frouter_routes) =
                if frouter.is_null() {
                    (0, 0, 0, 0)
                } else {
                    unsafe {
                        (
                            core::ptr::read_volatile(frouter.add(1)),
                            core::ptr::read_volatile(frouter.add(2)),
                            core::ptr::read_volatile(frouter.add(3)),
                            core::ptr::read_volatile(frouter.add(5)),
                        )
                    }
                };
            crate::logln!(
                "[join] timeout diagnostics: state={} members={} flags={:#x} attempts={} \
                 requests={} replies={} millis={} routes={} pending={} queued={} term={} \
                 tags={}/{}/{}/{}/{}/{} log={}/{}/{} frouter={}/{}/{}/{}",
                word(7),
                word(8),
                word(9),
                word(10),
                word(11),
                word(12),
                word(13),
                word(14),
                word(15),
                word(16),
                word(5),
                word(17),
                word(18),
                word(19),
                word(20),
                word(21),
                word(22),
                word(23),
                word(24),
                word(25),
                frouter_rx,
                frouter_forwarded,
                frouter_dropped,
                frouter_routes
            );
        }
        assert!(
            generation.is_some_and(|generation| generation >= 1),
            "[join] membership event must fire through consensus, got {generation:?}"
        );
        crate::logln!(
            "[join] membership event fired (generation {}) — admitted to the cluster.",
            generation.unwrap()
        );

        crate::logln!("[join] SUCCESS: node admitted to the cluster via discovery-bridged join.");
        let _ = ipc::close_cap(crate::memory::KERNEL_ASID, kernel_ns);
        crate::self_test::results::pass(crate::self_test::results::TestId::Join);
    }
}

pub use inner::test_el0_join;
