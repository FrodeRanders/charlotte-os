#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    boxed::Box,
    string::{
        String,
        ToString,
    },
    sync::Arc,
    vec::Vec,
};

use catten_graft::{
    charlotte::CharlotteTransport,
    transport::{
        AppendEntriesRpc,
        InstallSnapshotRpc,
        RaftTransport,
        RpcCompletion,
    },
    log_store::{
        InMemoryLogStore,
        InMemoryPersistentStateStore,
        LogStore,
        PersistentStateStore,
    },
    membership::ClusterConfiguration,
    node::RaftNode,
    types::{
        NodeState,
        Peer,
    },
    wire::{
        RAFT_RPC_MEMORY_SIZE,
        SCRATCH_VADDR,
        decode_append_request,
        decode_snapshot_request,
        decode_vote_request,
        encode_append_response,
        encode_snapshot_response,
        encode_vote_response,
    },
};
use catten_rt::{
    Context,
    ManifestValue,
    config,
    manifest_key,
};
use catten_services::{
    disk_raft::{
        DiskLogStore,
        DiskPersistentStateStore,
    },
    disco,
    dns,
    ns,
    raft,
    relmsg,
    relmsg_transport::{
        TAG_JOIN_REPLY,
        TAG_JOIN_REQUEST,
        RelmsgRaftTransport,
        decode_join_reply,
        decode_join_request,
        encode_join_reply,
        encode_join_request,
    },
    sleep_ms,
};
use charlotte_protocol_disco::{
    ROLE_LEADER,
    parse_cluster_answer,
};
use charlotte_protocol_msg::unpack_address_and_len;
use catten_syscall::{
    IpcRights,
    ipc_reply_poll_with_memory,
    cq_read,
    cq_wait_timeout,
    ipc_close,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_reply_poll,
    ipc_reply_wait,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map,
    memory_unmap,
    submit_detached_timer,
    thread_exit,
};

const LOOP_TICK_MS: u64 = 25;
/// Scratch for inbound relmsg frames (distinct from the RPC memory scratch).
const RX_SCRATCH: usize = 0x0000_0000_0082_1000;
/// Cadence for re-querying discovery for MAC routes and cluster posture.
const DISCO_QUERY_MS: u64 = 2_000;
const RAFT_TIMER_COOKIE: u64 = 0x5241_4654_5449_434b;

fn fatal(stage: u64) -> ! {
    catten_syscall::el0_log(0x5241_4654, stage);
    unsafe { thread_exit() }
}

unsafe fn wait_reply_2(call: u64) -> Option<(i64, u64)> {
    // Keep the capability in memory across the multi-register reply-poll
    // syscall. This guards against the returned x1 result being confused
    // with the x1 input capability by aggressive inlining/register reuse.
    let saved_call = call;
    let call_cap = unsafe { core::ptr::read_volatile(&saved_call) };
    let (status, result, connection) = ipc_reply_wait(call_cap);
    ipc_close(unsafe { core::ptr::read_volatile(&saved_call) });
    (status == 0).then_some((result as i64, connection))
}

fn write_payload_to_mem(payload: &[u8]) -> Option<u64> {
    if payload.len() > RAFT_RPC_MEMORY_SIZE {
        return None;
    }
    let cap = memory_alloc(1);
    if cap == 0 {
        return None;
    }
    if memory_map(cap, SCRATCH_VADDR, true) != 0 {
        memory_close(cap);
        return None;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(payload.as_ptr(), SCRATCH_VADDR as *mut u8, payload.len());
    }
    memory_unmap(cap);
    Some(cap)
}

fn read_payload_from_mem(cap: u64, length: u64) -> Option<Vec<u8>> {
    let length = usize::try_from(length).ok()?;
    if cap == 0 || length > RAFT_RPC_MEMORY_SIZE {
        if cap != 0 {
            memory_close(cap);
        }
        return None;
    }
    let map_status = memory_map(cap, SCRATCH_VADDR, false);
    if map_status != 0 {
        memory_close(cap);
        return None;
    }
    let value = unsafe { core::slice::from_raw_parts(SCRATCH_VADDR as *const u8, length).to_vec() };
    memory_unmap(cap);
    memory_close(cap);
    Some(value)
}

fn reply_payload(reply: u64, payload: Result<Vec<u8>, catten_graft::wire::WireError>) {
    if let Ok(payload) = payload
        && let Some(memory) = write_payload_to_mem(&payload)
    {
        ipc_reply_move(reply, memory, payload.len() as i64);
        return;
    }
    ipc_reply(reply, -1);
}

fn poll_peer_discovery(
    ns_conn: u64,
    peer_id: &str,
    peer_name: u64,
    pending: &mut u64,
    transport: &CharlotteTransport,
) {
    if transport.has_peer(peer_id) {
        return;
    }
    if *pending == 0 {
        *pending = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, peer_name);
        return;
    }
    let (status, generation, connection) = ipc_reply_poll(*pending);
    match status {
        0 => {
            ipc_close(*pending);
            *pending = 0;
            if generation >= 1 && connection != 0 {
                transport.add_peer(peer_id, connection);
            }
        }
        1 => {}
        _ => {
            ipc_close(*pending);
            *pending = 0;
        }
    }
}

/// Best-effort cluster-event publishing state: the node's dns is the
/// replicated event broker, so this node can publish conditions that
/// dependent activities anywhere in the cluster wait on. Fires are
/// fire-and-forget with retry; no ordering or leader status is assumed.
struct EventFirer {
    dns_lookup: u64,
    dns_conn: u64,
    established_fired: bool,
    fired_members: alloc::vec::Vec<alloc::string::String>,
    pending_fire: u64,
    pending_name: alloc::vec::Vec<u8>,
}

impl EventFirer {
    fn new() -> Self {
        Self {
            dns_lookup: 0,
            dns_conn: 0,
            established_fired: false,
            fired_members: alloc::vec::Vec::new(),
            pending_fire: 0,
            pending_name: alloc::vec::Vec::new(),
        }
    }
}

fn fire_event(dns_conn: u64, event_name: &[u8]) -> u64 {
    let Some(memory) = write_payload_to_mem(event_name) else {
        return 0;
    };
    let call = catten_syscall::ipc_scalar_call_move(
        dns_conn,
        dns::OP_EVENT_FIRE,
        event_name.len() as u64,
        memory,
    );
    if call == 0 {
        catten_syscall::memory_close(memory);
    }
    call
}

/// The raft service's transport is a hybrid: manifest-configured peers
/// (same-node deployments, e.g. the raft self-test) connect through the name
/// service, while identity-based peers on the network segment route by MAC
/// through the reliable-message layer. The graft node sees one transport.
struct ServiceRaftTransport {
    ns: Arc<CharlotteTransport>,
    mac: Arc<RelmsgRaftTransport>,
}

impl RaftTransport for ServiceRaftTransport {
    fn set_current_millis(&self, current_millis: u64) {
        self.ns.set_current_millis(current_millis);
        self.mac.set_current_millis(current_millis);
    }

    fn send_vote_request(
        &self,
        peer: &Peer,
        term: u64,
        candidate_id: &str,
        last_log_index: u64,
        last_log_term: u64,
    ) {
        if self.ns.has_peer(&peer.id) {
            self.ns.send_vote_request(peer, term, candidate_id, last_log_index, last_log_term);
        } else {
            self.mac.send_vote_request(peer, term, candidate_id, last_log_index, last_log_term);
        }
    }

    fn send_append_entries(&self, rpc: AppendEntriesRpc<'_>) {
        if self.ns.has_peer(&rpc.peer.id) {
            self.ns.send_append_entries(rpc);
        } else {
            self.mac.send_append_entries(rpc);
        }
    }

    fn send_install_snapshot(&self, rpc: InstallSnapshotRpc<'_>) {
        if self.ns.has_peer(&rpc.peer.id) {
            self.ns.send_install_snapshot(rpc);
        } else {
            self.mac.send_install_snapshot(rpc);
        }
    }

    fn broadcast_heartbeat_complete(&self) {
        self.ns.broadcast_heartbeat_complete();
        self.mac.broadcast_heartbeat_complete();
    }

    fn poll_completions(&self) -> Vec<RpcCompletion> {
        let mut completions = self.ns.poll_completions();
        completions.extend(self.mac.poll_completions());
        completions
    }
}

/// Drive an inbound Raft RPC (decoded from a relmsg frame) into the node and
/// reply to the source MAC. Mirrors the dns's relmsg raft transport usage.
fn drive_inbound(
    node: &mut RaftNode,
    transport: &RelmsgRaftTransport,
    source_mac: [u8; 6],
    inbound: catten_services::relmsg_transport::InboundRpc,
    millis: u64,
) {
    match inbound {
        catten_services::relmsg_transport::InboundRpc::VoteRequest(request) => {
            let response = node.handle_vote_request(request, millis);
            if let Ok(payload) = catten_graft::wire::encode_vote_response(&response) {
                transport.send_response(source_mac, catten_services::relmsg_transport::TAG_VOTE_RESPONSE, payload);
            }
        }
        catten_services::relmsg_transport::InboundRpc::AppendEntries(request) => {
            let response = node.handle_append_entries(request, millis);
            if let Ok(payload) = catten_graft::wire::encode_append_response(&response) {
                transport.send_response(source_mac, catten_services::relmsg_transport::TAG_APPEND_RESPONSE, payload);
            }
        }
        catten_services::relmsg_transport::InboundRpc::InstallSnapshot(request) => {
            let response = node.handle_install_snapshot(request, millis);
            if let Ok(payload) = catten_graft::wire::encode_snapshot_response(&response) {
                transport.send_response(source_mac, catten_services::relmsg_transport::TAG_SNAPSHOT_RESPONSE, payload);
            }
        }
    }
}

const NODE_ID_KEY: u64 = manifest_key(b"node-id");
const PEER_ID_KEY: u64 = manifest_key(b"peer-id");
const ELECTION_KEY: u64 = manifest_key(b"elect-ms");
const CLUSTER_KEY: u64 = manifest_key(b"cluster");
const STORAGE_KEY: u64 = manifest_key(b"storage");

const STORAGE_MEMORY: u64 = 0;
const STORAGE_OPTIONAL: u64 = 1;
const STORAGE_REQUIRED: u64 = 2;

fn persistent_namespace(cluster_id: &[u8], node_id: &[u8]) -> u64 {
    // Stable FNV-1a over the cluster/node tuple. This is an object-store
    // namespace, not a security boundary; ownership policy remains external.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in
        cluster_id.iter().copied().chain(core::iter::once(0xff)).chain(node_id.iter().copied())
    {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(0, 1);
    let election_timeout_ms = match ctx.manifest_value(ELECTION_KEY) {
        Some(ManifestValue::Unsigned(value)) => value,
        _ => 150,
    };
    let cluster_id = match ctx.manifest_value(CLUSTER_KEY) {
        Some(ManifestValue::Bytes(bytes)) if !bytes.is_empty() => bytes,
        _ => b"default",
    };
    let storage_policy = match ctx.manifest_value(STORAGE_KEY) {
        Some(ManifestValue::Unsigned(STORAGE_MEMORY)) => STORAGE_MEMORY,
        Some(ManifestValue::Unsigned(STORAGE_OPTIONAL)) => STORAGE_OPTIONAL,
        Some(ManifestValue::Unsigned(STORAGE_REQUIRED)) => STORAGE_REQUIRED,
        _ => STORAGE_MEMORY,
    };

    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => fatal(1),
    };
    config::write::<u32>(0, 2);

    // The node id is the raft identity other nodes use to reach this node
    // (service name `raft-{node_id}`). Without an explicit manifest override
    // it derives from the node identity (the same name discovery advertises),
    // so a discovered peer can always locate this node's raft endpoint.
    // The identity is created by the discovery service (which knows the NIC
    // MAC); this service may boot first, so the read retries briefly before
    // falling back to the bootable "r1" placeholder.
    let node_id: String = match ctx.manifest_value(NODE_ID_KEY) {
        Some(ManifestValue::Bytes(bytes)) => {
            let id = core::str::from_utf8(bytes).unwrap_or("r1");
            if id.is_empty() {
                "r1".to_string()
            } else {
                id.to_string()
            }
        }
        _ => {
            let mut identity_retries = 120u32;
            loop {
                if let Some(identity) = catten_services::node_identity::NodeIdentity::load_or_create(
                    ns_conn,
                    cluster_id,
                    None,
                ) {
                    break identity.name_str().to_string();
                }
                if identity_retries == 0 {
                    break "r1".to_string();
                }
                identity_retries -= 1;
                sleep_ms(250);
            }
        }
    };

    let endpoint = ipc_endpoint_create(raft::INTERFACE, raft::VERSION, 8);
    if endpoint == 0 {
        fatal(2);
    }
    if ipc_endpoint_bind_cq(endpoint, 0) != 0 {
        fatal(5);
    }
    config::write::<u32>(0, 3);

    let name_u64 = catten_services::raft_name(node_id.as_bytes());
    catten_syscall::el0_log(0x5241_4654, name_u64 | 0x0000_0000_0100_0000);

    let register = ipc_scalar_call_connection(
        ns_conn,
        ns::OP_REGISTER,
        name_u64,
        endpoint,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if register == 0 {
        fatal(3);
    }
    config::write::<u32>(0, 4);

    let (generation, _) = unsafe { wait_reply_2(register).unwrap_or((-1, 0)) };
    if generation < 1 {
        fatal(4);
    }
    config::write::<u32>(4, generation as u32);

    // Storage is launch policy. The ordinary local Raft service test uses
    // memory and remains independent of NVMe. A durability deployment can
    // require the object store and wait for its name-service registration;
    // optional mode may fall back, but never does so silently.
    let namespace = persistent_namespace(cluster_id, node_id.as_bytes());
    let disk_stores = if storage_policy == STORAGE_MEMORY {
        None
    } else {
        let wait_for_service = storage_policy == STORAGE_REQUIRED;
        match (
            DiskLogStore::new(ns_conn, namespace, wait_for_service),
            DiskPersistentStateStore::new(ns_conn, namespace, wait_for_service),
        ) {
            (Some(log), Some(state)) => Some((log, state)),
            _ if storage_policy == STORAGE_REQUIRED => fatal(9),
            _ => None,
        }
    };
    let (log_store, persistent_store, durable): (
        Box<dyn LogStore>,
        Box<dyn PersistentStateStore>,
        bool,
    ) = match disk_stores {
        Some((log, state)) => (Box::new(log), Box::new(state), true),
        None => (
            Box::new(InMemoryLogStore::new()),
            Box::new(InMemoryPersistentStateStore::new()),
            false,
        ),
    };
    config::write::<u32>(24, durable as u32);
    let ns_transport = Arc::new(CharlotteTransport::new());
    let mac_transport = Arc::new(RelmsgRaftTransport::new(0));
    let transport: Arc<dyn RaftTransport> = Arc::new(ServiceRaftTransport {
        ns: ns_transport.clone(),
        mac: mac_transport.clone(),
    });

    let mut peers = Vec::new();
    let me = Peer::voter(node_id.to_string(), name_u64);
    peers.push(me.clone());

    let mut peer_specs: Vec<(String, u64, u64)> = Vec::new();
    for entry in ctx.manifest().filter(|entry| entry.key == PEER_ID_KEY) {
        let ManifestValue::Bytes(peer_bytes) = entry.value else {
            continue;
        };
        let peer_id = core::str::from_utf8(peer_bytes).unwrap_or("");
        if peer_id.is_empty() {
            continue;
        }

        let peer_name = catten_services::raft_name(peer_id.as_bytes());
        peers.push(Peer::voter(peer_id.to_string(), peer_name));
        peer_specs.push((peer_id.to_string(), peer_name, 0));
    }

    config::write::<u32>(0, 5);

    let mut node = RaftNode::new(catten_graft::node::RaftNodeConfig {
        me,
        timeout_millis: election_timeout_ms,
        log_store,
        persistent_state: persistent_store,
        state_machine: None,
        cluster_configuration: ClusterConfiguration::stable(peers),
        transport: transport.clone(),
        current_millis: 0,
        snapshot_min_entries: 64,
        snapshot_chunk_bytes: 3000,
    });
    config::write::<u32>(20, node.current_term as u32);
    config::write::<u32>(0, 6);

    let mut served: u32 = 0;

    let cq = ctx.completion_queue_layout();
    let mut timer_armed = submit_detached_timer(LOOP_TICK_MS, 0, RAFT_TIMER_COOKIE) != u64::MAX;

    let mut events = EventFirer::new();

    // Network-cluster state: the node locates peers by MAC through discovery
    // and exchanges raft traffic over the reliable-message layer. None of
    // this consults the local name service for remote participants — before
    // membership the only addresses that exist are the MACs discovery saw.
    let mut relmsg_lookup: u64 = 0;
    let mut relmsg_conn: u64 = 0;
    let mut disco_lookup: u64 = 0;
    let mut disco_conn: u64 = 0;
    let mut disco_query: u64 = 0;
    let mut next_disco_query_ms: u64 = 0;
    let mut recv_pending: u64 = 0;
    let mut join_request_pending = false;
    let mut join_accepted = false;

    loop {
        // Endpoint readiness and transport completions wake this reactor
        // immediately. The bounded wait itself supplies Raft's periodic clock,
        // avoiding a separate detached-timer completion and wake path.
        let (_, timed_out) = cq_wait_timeout(
            1,
            if timer_armed {
                1_000
            } else {
                LOOP_TICK_MS
            },
            0,
        );
        let mut tick_due = !timer_armed && timed_out != 0;
        while let Some(completion) = unsafe { cq_read(cq.base, cq.entries) } {
            if completion.cookie == RAFT_TIMER_COOKIE {
                tick_due = true;
                timer_armed = false;
            }
        }

        let completed = node.poll_transport(node.millis());
        if completed > 0 {
            config::write::<u32>(16, completed as u32);
        }

        // Keep one deferred name-service lookup outstanding for each missing
        // peer. Registration completes that call; the reactor only polls the
        // existing call and never creates retry storms or blocks on a peer.
        // Membership is replicated, so discovery must follow the committed
        // configuration rather than remaining frozen at the boot manifest.
        // Connect targets are the committed membership plus peers admitted
        // by committed JOIN entries that are pending promotion: the leader
        // must replicate those joiners up to their fences before they enter
        // the joint configuration.
        let mut active_peer_ids: Vec<String> = Vec::new();
        for peer in node.cluster_configuration.all_members() {
            if !active_peer_ids.contains(&peer.id) {
                active_peer_ids.push(peer.id.clone());
            }
        }
        for peer in node.pending_joiners() {
            if !active_peer_ids.contains(&peer.id) {
                active_peer_ids.push(peer.id.clone());
            }
        }
        peer_specs.retain(|(peer_id, _, pending)| {
            if active_peer_ids.contains(peer_id) {
                true
            } else {
                if *pending != 0 {
                    ipc_close(*pending);
                }
                ns_transport.remove_peer(peer_id);
                false
            }
        });
        for peer_id in &active_peer_ids {
            if peer_id == &node.me.id || peer_specs.iter().any(|spec| &spec.0 == peer_id) {
                continue;
            }
            let peer_name = node
                .cluster_configuration
                .all_members()
                .iter()
                .find(|peer| &peer.id == peer_id)
                .map(|peer| peer.service_name)
                .or_else(|| {
                    node.pending_joiners()
                        .iter()
                        .find(|peer| &peer.id == peer_id)
                        .map(|peer| peer.service_name)
                })
                .unwrap_or_else(|| {
                    catten_services::raft_name(peer_id.as_bytes())
                });
            peer_specs.push((peer_id.clone(), peer_name, 0));
        }
        for (peer_id, peer_name, pending) in &mut peer_specs {
            poll_peer_discovery(ns_conn, peer_id, *peer_name, pending, &ns_transport);
        }

        // --- Network-cluster connections (relmsg + disco) ---
        // The relmsg is the MAC-addressed frame transport to other nodes; the
        // disco supplies the MAC routes and cluster posture. Both are resolved
        // through the local name service (these services live on this node).
        if relmsg_conn == 0 && relmsg_lookup == 0 {
            relmsg_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, relmsg::NAME);
        }
        if relmsg_lookup != 0 {
            let (status, generation, conn) = ipc_reply_poll(relmsg_lookup);
            if status == 0 {
                ipc_close(relmsg_lookup);
                relmsg_lookup = 0;
                if generation >= 1 && conn != 0 {
                    relmsg_conn = conn;
                    mac_transport.set_relmsg_conn(conn);
                }
            } else if status != 1 {
                ipc_close(relmsg_lookup);
                relmsg_lookup = 0;
            }
        }
        if disco_conn == 0 && disco_lookup == 0 {
            disco_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, disco::NAME);
        }
        if disco_lookup != 0 {
            let (status, generation, conn) = ipc_reply_poll(disco_lookup);
            if status == 0 {
                ipc_close(disco_lookup);
                disco_lookup = 0;
                if generation >= 1 && conn != 0 {
                    disco_conn = conn;
                }
            } else if status != 1 {
                ipc_close(disco_lookup);
                disco_lookup = 0;
            }
        }

        // --- MAC routes + cluster posture via discovery ---
        // Query the disco for every node on the segment: their MACs are the
        // only addresses that exist pre-membership, and their reported roles
        // identify the leader to apply to (or a follower whose leader hint is
        // the redirect).
        if disco_conn != 0 && disco_query == 0 && node.millis() >= next_disco_query_ms {
            next_disco_query_ms = node.millis() + DISCO_QUERY_MS;
            disco_query = ipc_scalar_call(disco_conn, disco::OP_CLUSTER_STATUS, 0);
        }
        if disco_query != 0 {
            let (status, _result, _returned_connection, memory) =
                ipc_reply_poll_with_memory(disco_query);
            if status == 0 {
                ipc_close(disco_query);
                disco_query = 0;
                if memory != 0 {
                    if memory_map(memory, RX_SCRATCH, false) == 0 {
                        let bytes = unsafe {
                            core::slice::from_raw_parts(RX_SCRATCH as *const u8, 4096)
                        };
                        if let Some((_self_role, _self_raft_id, _self_leader_id, peers)) =
                            parse_cluster_answer(bytes)
                        {
                            // MAC routes for every discovered peer: the hybrid
                            // transport sends via MAC for anyone not reachable
                            // through the local name service.
                            for (mac, _role, raft_id, _leader_id) in &peers {
                                if !raft_id.is_empty() {
                                    mac_transport.add_peer(
                                        core::str::from_utf8(raft_id).unwrap_or(""),
                                        *mac,
                                    );
                                }
                            }
                            // Auto-join: a single-node leader whose id is
                            // lexicographically larger than a discovered
                            // leader's applies for membership of that cluster
                            // (the smaller id is the anchor and waits). The
                            // join request is a MAC-addressed frame — no local
                            // lookup of the peer exists or is needed.
                            if !join_accepted
                                && !join_request_pending
                                && node.state == NodeState::Leader
                                && node.cluster_configuration.all_members().len() == 1
                            {
                                if let Some((_mac, _role, raft_id, _leader_id)) = peers
                                    .iter()
                                    .find(|(_, role, raft_id, _)| {
                                        *role == ROLE_LEADER
                                            && !raft_id.is_empty()
                                            && raft_id.as_slice() != node.me.id.as_bytes()
                                            && node.me.id.as_bytes() > raft_id.as_slice()
                                    })
                                {
                                    let target =
                                        core::str::from_utf8(raft_id).unwrap_or("");
                                    if let Some(payload) = encode_join_request(
                                        node.me.id.as_bytes(),
                                        name_u64,
                                    ) {
                                        mac_transport.send_message(target, TAG_JOIN_REQUEST, payload);
                                        join_request_pending = true;
                                        catten_syscall::el0_log(
                                            0x5241_4654,
                                            0x4a4f_494e,
                                        );
                                    }
                                }
                            }
                        }
                        memory_unmap(memory);
                    }
                    memory_close(memory);
                }
            } else if status != 1 {
                ipc_close(disco_query);
                disco_query = 0;
            }
        }

        // --- Inbound frames over relmsg ---
        if relmsg_conn != 0 && recv_pending == 0 {
            recv_pending = ipc_scalar_call(relmsg_conn, relmsg::OP_RECV, 0);
        }
        if recv_pending != 0 {
            let (recv_status, result, _returned_connection, memory) =
                ipc_reply_poll_with_memory(recv_pending);
            if recv_status == 0 {
                ipc_close(recv_pending);
                recv_pending = 0;
                if memory != 0 {
                    let (source_mac, len) = unpack_address_and_len(result);
                    if memory_map(memory, RX_SCRATCH, false) == 0 {
                        let frame = unsafe {
                            core::slice::from_raw_parts(RX_SCRATCH as *const u8, len as usize)
                        };
                        match frame.first().copied() {
                            Some(tag) if (1..=6).contains(&tag) => {
                                if let Some(inbound) =
                                    mac_transport.decode_inbound(&source_mac, frame)
                                {
                                    let millis = node.millis();
                                    drive_inbound(
                                        &mut node,
                                        &mac_transport,
                                        source_mac,
                                        inbound,
                                        millis,
                                    );
                                }
                            }
                            Some(TAG_JOIN_REQUEST) => {
                                if let Some((joiner_id, service_name)) =
                                    decode_join_request(frame)
                                {
                                    let (accepted, detail) = if node.state == NodeState::Leader {
                                        let peer = Peer::voter(
                                            core::str::from_utf8(joiner_id)
                                                .unwrap_or("")
                                                .to_string(),
                                            service_name,
                                        );
                                        match node.submit_join(peer, node.millis()) {
                                            Ok(index) => (index, 0u64),
                                            Err(code) => (0, code.unsigned_abs()),
                                        }
                                    } else {
                                        (0, 0xff)
                                    };
                                    mac_transport.send_response(
                                        source_mac,
                                        TAG_JOIN_REPLY,
                                        encode_join_reply(accepted),
                                    );
                                    catten_syscall::el0_log(
                                        0x5241_4654,
                                        0x4a52_4551 | (accepted << 16) | (detail << 32),
                                    );
                                }
                            }
                            Some(TAG_JOIN_REPLY) => {
                                if let Some(index) = decode_join_reply(frame) {
                                    join_request_pending = false;
                                    if index > 0 {
                                        join_accepted = true;
                                        catten_syscall::el0_log(0x5241_4654, 0x4a4f_494e_41);
                                    }
                                }
                            }
                            _ => {}
                        }
                        memory_unmap(memory);
                    }
                    memory_close(memory);
                }
            }
        }
        mac_transport.drain_outbound();
        mac_transport.reap_acks();

        // Drain inbound Raft traffic after processing the timer tick.
        loop {
            let message = ipc_recv(endpoint);
            if message.status == ipc_status::NO_MESSAGE {
                break;
            }
            if message.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() };
            }
            if !message.is_ok() {
                break;
            }

            served += 1;
            config::write::<u32>(12, served);

            match message.opcode {
                raft::OP_VOTE_REQUEST => {
                    let request = read_payload_from_mem(message.memory, message.arg0)
                        .and_then(|payload| decode_vote_request(&payload).ok());
                    if let Some(request) = request {
                        let response = node.handle_vote_request(request, node.millis());
                        if message.reply != 0 {
                            reply_payload(message.reply, encode_vote_response(&response));
                        }
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }

                raft::OP_APPEND_ENTRIES => {
                    let request = read_payload_from_mem(message.memory, message.arg0)
                        .and_then(|payload| decode_append_request(&payload).ok());
                    if let Some(request) = request {
                        let response = node.handle_append_entries(request, node.millis());
                        if message.reply != 0 {
                            reply_payload(message.reply, encode_append_response(&response));
                        }
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }

                raft::OP_INSTALL_SNAPSHOT => {
                    let request = read_payload_from_mem(message.memory, message.arg0)
                        .and_then(|payload| decode_snapshot_request(&payload).ok());
                    if let Some(request) = request {
                        let response = node.handle_install_snapshot(request, node.millis());
                        if message.reply != 0 {
                            reply_payload(message.reply, encode_snapshot_response(&response));
                        }
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }

                raft::OP_STATUS => {
                    let status: u32 = match node.state {
                        NodeState::Follower => 1,
                        NodeState::Candidate => 2,
                        NodeState::Leader => 3,
                    };
                    let result = (status as i64)
                        | ((node.current_term as i64) << 8)
                        | ((node.commit_index as i64) << 32);
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                raft::OP_CLUSTER_STATUS => {
                    catten_syscall::el0_log(0x5241_4654, 0x5555);
                    let status: u32 = match node.state {
                        NodeState::Follower => 1,
                        NodeState::Candidate => 2,
                        NodeState::Leader => 3,
                    };
                    let mut buf = [0u8; 256];
                    let leader_id = node.known_leader_id.clone().unwrap_or_default();
                    let len = raft::build_cluster_status(
                        &mut buf,
                        status,
                        node.current_term,
                        node.commit_index,
                        node.cluster_configuration.all_members().len() as u32,
                        leader_id.as_bytes(),
                        node.me.id.as_bytes(),
                    );
                    if let Some(len) = len
                        && let Some(memory) = write_payload_to_mem(&buf[..len])
                    {
                        catten_syscall::el0_log(0x5241_4654, 0x5556);
                        ipc_reply_move(message.reply, memory, len as i64);
                    } else if message.reply != 0 {
                        catten_syscall::el0_log(0x5241_4654, 0x5557);
                        ipc_reply(message.reply, -1);
                    }
                }

                raft::OP_ADD_SERVER => {
                    let spec: Option<(String, u64, bool)> =
                        read_payload_from_mem(message.memory, message.arg0)
                            .and_then(|payload| {
                                let (id, service_name, learner) =
                                    raft::decode_peer_spec(&payload)?;
                                core::str::from_utf8(id)
                                    .ok()
                                    .map(|id| (id.to_string(), service_name, learner))
                            });
                    match spec {
                        Some((id, service_name, learner)) => {
                            if id.is_empty() {
                                ipc_reply(message.reply, -1);
                            } else {
                                let peer = if learner {
                                    Peer::learner(id, service_name)
                                } else {
                                    Peer::voter(id, service_name)
                                };
                                match node.submit_join(peer, node.millis()) {
                                    Ok(index) => ipc_reply(message.reply, index as i64),
                                    Err(code) => ipc_reply(message.reply, code),
                                };
                            }
                        }
                        None => {
                            ipc_reply(message.reply, -1);
                        }
                    }
                }

                raft::OP_REMOVE_SERVER => {
                    let id = read_payload_from_mem(message.memory, message.arg0)
                        .and_then(|payload| {
                            if payload.is_empty() {
                                return None;
                            }
                            let id_len = payload[0] as usize;
                            if id_len == 0 || id_len > 255 || payload.len() < 1 + id_len {
                                return None;
                            }
                            core::str::from_utf8(&payload[1..1 + id_len])
                                .ok()
                                .map(|id| id.to_string())
                        });
                    match id {
                        Some(id) if node.state == NodeState::Leader => {
                            let members: Vec<Peer> = node
                                .cluster_configuration
                                .all_members()
                                .into_iter()
                                .cloned()
                                .filter(|peer| peer.id != id)
                                .collect();
                            if members.is_empty() {
                                // Refuse to decommission the last member.
                                ipc_reply(message.reply, raft::ERR_NOT_FOUND);
                            } else {
                                match node
                                    .submit_joint_configuration(members, node.millis())
                                {
                                    Ok(index) => ipc_reply(message.reply, index as i64),
                                    Err(code) => ipc_reply(message.reply, code),
                                };
                            }
                        }
                        Some(_) => {
                            ipc_reply(message.reply, raft::ERR_NOT_LEADER);
                        }
                        None => {
                            ipc_reply(message.reply, -1);
                        }
                    }
                }

                _ => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }
            }
        }

        if tick_due {
            node.set_millis(node.millis() + LOOP_TICK_MS);
            if node.check_timeout() {
                node.start_election(node.millis());
            }
            timer_armed = submit_detached_timer(LOOP_TICK_MS, 0, RAFT_TIMER_COOKIE) != u64::MAX;
        }

        if node.state == NodeState::Leader {
            node.broadcast_heartbeat(node.millis());
        }

        // --- Cluster-event publishing (best-effort) ---
        // The dns is the replicated event broker: conditions published here
        // (cluster established, membership.{id} after a JOIN finalizes) are
        // committed through consensus and waited on by dependent activities,
        // so no caller polls or assumes ordering.
        if events.dns_lookup == 0 && events.dns_conn == 0 {
            events.dns_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, dns::NAME);
        }
        if events.dns_lookup != 0 {
            let (status, generation, conn) = ipc_reply_poll(events.dns_lookup);
            if status == 0 {
                ipc_close(events.dns_lookup);
                events.dns_lookup = 0;
                if generation >= 1 && conn != 0 {
                    events.dns_conn = conn;
                }
            } else if status != 1 {
                ipc_close(events.dns_lookup);
                events.dns_lookup = 0;
            }
        }
        if events.dns_conn != 0 && events.pending_fire == 0 {
            if node.state == NodeState::Leader && !events.established_fired {
                events.pending_name = b"event:established".to_vec();
                events.pending_fire = fire_event(events.dns_conn, &events.pending_name);
            } else {
                for peer in node.cluster_configuration.all_members() {
                    if peer.id != node.me.id
                        && !events.fired_members.contains(&peer.id)
                    {
                        events.pending_name =
                            alloc::format!("event:membership:{}", peer.id).into_bytes();
                        events.pending_fire = fire_event(events.dns_conn, &events.pending_name);
                        break;
                    }
                }
            }
        }
        if events.pending_fire != 0 {
            let (status, result, _conn) = ipc_reply_poll(events.pending_fire);
            if status == 0 {
                ipc_close(events.pending_fire);
                events.pending_fire = 0;
                let name = core::mem::take(&mut events.pending_name);
                if result > 0 {
                    if name == b"event:established" {
                        events.established_fired = true;
                    } else if let Some(id) = name.strip_prefix(b"event:membership:") {
                        events.fired_members.push(
                            core::str::from_utf8(id).unwrap_or("").to_string(),
                        );
                    }
                }
                // On failure (e.g. ERR_NOT_LEADER) the next iteration retries
                // the same pending event.
            }
        }

        // Publish one coherent observation after all state transitions in
        // this reactor iteration. Write the term first so a verifier that
        // observes Leader cannot still see the preceding election term.
        config::write::<u32>(20, node.current_term as u32);
        config::write::<u32>(
            8,
            match node.state {
                NodeState::Candidate => 2,
                NodeState::Leader => 3,
                NodeState::Follower => 1,
            },
        );
    }
}

catten_rt::entry!(main);
