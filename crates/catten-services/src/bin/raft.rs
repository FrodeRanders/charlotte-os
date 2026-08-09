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
    log_store::{
        InMemoryLogStore,
        InMemoryPersistentStateStore,
        LogStore,
        PersistentStateStore,
    },
    membership::ClusterConfiguration,
    node::RaftNode,
    transport::{
        AppendEntriesRpc,
        InstallSnapshotRpc,
        RaftTransport,
        RpcCompletion,
    },
    types::{
        NodeState,
        Peer,
    },
    wire::{
        RAFT_RPC_MEMORY_SIZE,
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
    disco,
    disk_raft::{
        DiskLogStore,
        DiskPersistentStateStore,
    },
    dns,
    frouter,
    net,
    ns,
    raft,
    relmsg_transport::{
        RelmsgRaftTransport,
        TAG_JOIN_REPLY,
        TAG_JOIN_REQUEST,
        decode_join_reply,
        decode_join_request,
        encode_join_reply,
        encode_join_request,
    },
    sleep_ms,
};
use catten_syscall::{
    IpcRights,
    cq_read,
    cq_wait_timeout,
    ipc_close,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_reply_poll,
    ipc_reply_poll_with_memory,
    ipc_reply_wait,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
    submit_detached_timer,
    thread_exit,
};
use charlotte_protocol_disco::{
    ROLE_LEADER,
    parse_cluster_answer,
};

const LOOP_TICK_MS: u64 = 25;
/// Scratch for inbound relmsg frames (distinct from the RPC memory scratch).
/// Cadence for re-querying discovery for MAC routes and cluster posture.
const DISCO_QUERY_MS: u64 = 2_000;
const JOIN_RETRY_MS: u64 = 1_000;
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
    let (scratch_vaddr_1_map_status, scratch_vaddr_1) = memory_map_any(cap, true);
    if scratch_vaddr_1_map_status != 0 {
        memory_close(cap);
        return None;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(payload.as_ptr(), scratch_vaddr_1 as *mut u8, payload.len());
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
    let (scratch_vaddr_2_map_status, scratch_vaddr_2) = memory_map_any(cap, false);
    let map_status = scratch_vaddr_2_map_status;
    if map_status != 0 {
        memory_close(cap);
        return None;
    }
    let value =
        unsafe { core::slice::from_raw_parts(scratch_vaddr_2 as *const u8, length).to_vec() };
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

/// Best-effort membership-event publishing state: the node's dns is the
/// replicated event broker, so dependent activities anywhere in the cluster
/// can wait on an admission committed by this Raft group. Fires are
/// fire-and-forget with retry.
struct EventFirer {
    dns_lookup: u64,
    dns_conn: u64,
    fired_members: alloc::vec::Vec<alloc::string::String>,
    pending_fire: u64,
    pending_name: alloc::vec::Vec<u8>,
}

impl EventFirer {
    fn new() -> Self {
        Self {
            dns_lookup: 0,
            dns_conn: 0,
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
                transport.send_response(
                    source_mac,
                    catten_services::relmsg_transport::TAG_VOTE_RESPONSE,
                    payload,
                );
            }
        }
        catten_services::relmsg_transport::InboundRpc::AppendEntries(request) => {
            let response = node.handle_append_entries(request, millis);
            if let Ok(payload) = catten_graft::wire::encode_append_response(&response) {
                transport.send_response(
                    source_mac,
                    catten_services::relmsg_transport::TAG_APPEND_RESPONSE,
                    payload,
                );
            }
        }
        catten_services::relmsg_transport::InboundRpc::InstallSnapshot(request) => {
            let response = node.handle_install_snapshot(request, millis);
            if let Ok(payload) = catten_graft::wire::encode_snapshot_response(&response) {
                transport.send_response(
                    source_mac,
                    catten_services::relmsg_transport::TAG_SNAPSHOT_RESPONSE,
                    payload,
                );
            }
        }
    }
}

const NODE_ID_KEY: u64 = manifest_key(b"node-id");
const PEER_ID_KEY: u64 = manifest_key(b"peer-id");
const ELECTION_KEY: u64 = manifest_key(b"elect-ms");
const CLUSTER_KEY: u64 = manifest_key(b"cluster");
const STORAGE_KEY: u64 = manifest_key(b"storage");
const NETWORK_KEY: u64 = manifest_key(b"network");

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
    // Network participation is launch policy, not an accidental side effect
    // of starting any Raft instance. Several independent Raft groups may run
    // on one node; only the node-cluster instance may own the well-known
    // Ethernet ingress name used by the frame router.
    let network_enabled = matches!(
        ctx.manifest_value(NETWORK_KEY),
        Some(ManifestValue::Unsigned(value)) if value != 0
    );

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
                    ns_conn, cluster_id, None,
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

    // Register the well-known frame name so the frame demultiplexer routes
    // this service's EtherType to its endpoint (the OP_FRAME ingress). The
    // frouter retries the lookup, so a later registration is fine.
    if network_enabled {
        let frame_register = ipc_scalar_call_connection(
            ns_conn,
            ns::OP_REGISTER,
            raft::FRAME_NAME,
            endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        );
        if frame_register == 0 {
            fatal(10);
        }
        let (frame_generation, returned_connection) =
            unsafe { wait_reply_2(frame_register).unwrap_or((-1, 0)) };
        if returned_connection != 0 {
            ipc_close(returned_connection);
        }
        if frame_generation < 1 {
            fatal(11);
        }
    }

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
    // Heartbeats must be frequent enough to preserve leadership but must not
    // run at reactor speed. This service shares the physical NIC/frouter
    // with DNS Raft and application traffic; an unconditional broadcast on
    // every loop can keep one stop-and-wait frame permanently in flight and
    // starve those other protocols under TCG.
    let heartbeat_interval_ms = (election_timeout_ms / 4).clamp(25, 250);
    let mut last_heartbeat_broadcast = 0u64;

    let cq = ctx.completion_queue_layout();
    let mut timer_armed = submit_detached_timer(LOOP_TICK_MS, 0, RAFT_TIMER_COOKIE) != u64::MAX;

    let mut events = EventFirer::new();

    // Network-cluster state: the node locates peers by MAC through discovery
    // and exchanges raft traffic over the reliable-message layer. None of
    // this consults the local name service for remote participants — before
    // membership the only addresses that exist are the MACs discovery saw.
    let mut net_lookup: u64 = 0;
    let mut net_conn: u64 = 0;
    let mut frouter_lookup: u64 = 0;
    let mut frouter_conn: u64 = 0;
    let mut disco_lookup: u64 = 0;
    let mut disco_conn: u64 = 0;
    let mut disco_query: u64 = 0;
    let mut next_disco_query_ms: u64 = 0;
    let mut join_request_pending = false;
    let mut join_retry_at_ms = 0;
    let mut join_accepted = false;
    let mut join_attempts = 0u32;
    let mut join_requests_received = 0u32;
    let mut join_replies_received = 0u32;
    let mut raft_tag_counts = [0u32; 6];

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
        // A successfully submitted detached timer is not proof that its CQ
        // completion will arrive. Keep the bounded CQ timeout as an
        // independent clock watchdog so one delayed/lost cookie cannot stop
        // elections and heartbeats forever.
        let mut tick_due = timed_out != 0;
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
                .unwrap_or_else(|| catten_services::raft_name(peer_id.as_bytes()));
            peer_specs.push((peer_id.clone(), peer_name, 0));
        }
        for (peer_id, peer_name, pending) in &mut peer_specs {
            poll_peer_discovery(ns_conn, peer_id, *peer_name, pending, &ns_transport);
        }

        // --- Network-cluster connections (net + frouter + disco) ---
        // The NIC is the MAC-addressed frame transport; the frouter demultiplexes
        // this service's own EtherType; the disco supplies the MAC routes and
        // cluster posture. All three are resolved through the local name service
        // (they live on this node).
        if network_enabled && net_conn == 0 && net_lookup == 0 {
            net_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, net::NAME);
        }
        if net_lookup != 0 {
            let (status, generation, conn) = ipc_reply_poll(net_lookup);
            if status == 0 {
                ipc_close(net_lookup);
                net_lookup = 0;
                if generation >= 1 && conn != 0 {
                    let status_call = ipc_scalar_call(conn, net::OP_STATUS, 0);
                    if status_call != 0 {
                        let (status, result, _cap) = catten_syscall::ipc_reply_wait(status_call);
                        catten_syscall::ipc_close(status_call);
                        let (link, mac) = if status == 0 {
                            charlotte_protocol_net::decode_status(result as i64)
                        } else {
                            (0, [0u8; 6])
                        };
                        if link != 0 {
                            net_conn = conn;
                            mac_transport.set_net_send(conn, mac, raft::ETHERTYPE);
                        }
                    }
                }
            } else if status != 1 {
                ipc_close(net_lookup);
                net_lookup = 0;
            }
        }
        if network_enabled && frouter_conn == 0 && frouter_lookup == 0 {
            frouter_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, frouter::NAME);
        }
        if frouter_lookup != 0 {
            let (status, generation, conn) = ipc_reply_poll(frouter_lookup);
            if status == 0 {
                ipc_close(frouter_lookup);
                frouter_lookup = 0;
                if generation >= 1 && conn != 0 {
                    frouter_conn = conn;
                }
            } else if status != 1 {
                ipc_close(frouter_lookup);
                frouter_lookup = 0;
            }
        }
        if network_enabled && disco_conn == 0 && disco_lookup == 0 {
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
        if join_request_pending && node.millis() >= join_retry_at_ms {
            join_request_pending = false;
            next_disco_query_ms = node.millis();
        }
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
                    let (rx_scratch_2_map_status, rx_scratch_2_vaddr) =
                        memory_map_any(memory, false);
                    if rx_scratch_2_map_status == 0 {
                        let bytes = unsafe {
                            core::slice::from_raw_parts(rx_scratch_2_vaddr as *const u8, 4096)
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
                            // Auto-join: the lexicographically larger
                            // single-node member applies to the smaller-id
                            // anchor. Do not require discovery's cached role
                            // to say "leader" before sending: posture can lag
                            // identity discovery, while the receiver itself is
                            // the authority that accepts only when it really
                            // is leader. A follower's leader hint is preferred
                            // when it names a route we have already learned.
                            if let Some((_mac, role, raft_id, leader_id)) = peers
                                .iter()
                                .filter(|(_, _, raft_id, _)| {
                                    !raft_id.is_empty()
                                        && raft_id.as_slice() != node.me.id.as_bytes()
                                        && node.me.id.as_bytes() > raft_id.as_slice()
                                })
                                .min_by(|left, right| left.2.cmp(&right.2))
                                && !join_accepted
                                && !join_request_pending
                                && node.cluster_configuration.all_members().len() == 1
                            {
                                let target_id = if *role != ROLE_LEADER
                                    && !leader_id.is_empty()
                                    && mac_transport
                                        .has_peer(core::str::from_utf8(leader_id).unwrap_or(""))
                                {
                                    leader_id.as_slice()
                                } else {
                                    raft_id.as_slice()
                                };
                                let target = core::str::from_utf8(target_id).unwrap_or("");
                                let may_apply = node.state == NodeState::Leader
                                    || (node.joining
                                        && node.joining_from.as_deref() == Some(target));
                                if may_apply
                                    && let Some(payload) =
                                        encode_join_request(node.me.id.as_bytes(), name_u64)
                                {
                                    if !node.joining {
                                        node.begin_joining(target.to_string(), node.millis());
                                    }
                                    mac_transport.send_message(target, TAG_JOIN_REQUEST, payload);
                                    join_request_pending = true;
                                    join_attempts = join_attempts.saturating_add(1);
                                    join_retry_at_ms = node.millis().saturating_add(JOIN_RETRY_MS);
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

        // Inbound frames arrive through the frouter's OP_FRAME ingress in the
        // endpoint drain below; outbound RPCs flush from the transport.
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

                raft::OP_FRAME => {
                    let frame_len = message.arg0 as usize;
                    if message.memory != 0 {
                        let (rx_scratch_map_status, rx_scratch_vaddr) =
                            memory_map_any(message.memory, false);
                        if rx_scratch_map_status == 0 && (15..=4096).contains(&frame_len) {
                            let frame = unsafe {
                                core::slice::from_raw_parts(
                                    rx_scratch_vaddr as *const u8,
                                    frame_len,
                                )
                            };
                            let mut source_mac = [0u8; 6];
                            source_mac.copy_from_slice(&frame[6..12]);
                            let Some((tag, payload)) =
                                catten_graft::wire::parse_tagged_payload(&frame[14..]).ok()
                            else {
                                memory_unmap(message.memory);
                                memory_close(message.memory);
                                if message.reply != 0 {
                                    ipc_reply(message.reply, -1);
                                }
                                continue;
                            };
                            match tag {
                                tag if (1..=6).contains(&tag) => {
                                    let counter = &mut raft_tag_counts[tag as usize - 1];
                                    *counter = counter.saturating_add(1);
                                    if let Some(inbound) = mac_transport.decode_inbound_parts(
                                        &source_mac,
                                        tag,
                                        payload,
                                    ) {
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
                                TAG_JOIN_REQUEST => {
                                    join_requests_received =
                                        join_requests_received.saturating_add(1);
                                    if let Some((joiner_id, service_name)) =
                                        decode_join_request(payload)
                                    {
                                        let accepted = if node.state == NodeState::Leader {
                                            let peer = Peer::voter(
                                                core::str::from_utf8(joiner_id)
                                                    .unwrap_or("")
                                                    .to_string(),
                                                service_name,
                                            );
                                            node.submit_join(peer, node.millis()).unwrap_or(0)
                                        } else {
                                            0
                                        };
                                        mac_transport.send_response(
                                            source_mac,
                                            TAG_JOIN_REPLY,
                                            encode_join_reply(accepted),
                                        );
                                    }
                                }
                                TAG_JOIN_REPLY => {
                                    join_replies_received = join_replies_received.saturating_add(1);
                                    if let Some(index) = decode_join_reply(payload) {
                                        join_request_pending = false;
                                        if index > 0 {
                                            join_accepted = true;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        memory_unmap(message.memory);
                        memory_close(message.memory);
                    }
                    if message.reply != 0 {
                        ipc_reply(message.reply, 0);
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
                        ipc_reply_move(message.reply, memory, len as i64);
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }

                raft::OP_ADD_SERVER => {
                    let spec: Option<(String, u64, bool)> =
                        read_payload_from_mem(message.memory, message.arg0).and_then(|payload| {
                            let (id, service_name, learner) = raft::decode_peer_spec(&payload)?;
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
                    let id =
                        read_payload_from_mem(message.memory, message.arg0).and_then(|payload| {
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
                                .filter(|peer| peer.id != id)
                                .cloned()
                                .collect();
                            if members.is_empty() {
                                // Refuse to decommission the last member.
                                ipc_reply(message.reply, raft::ERR_NOT_FOUND);
                            } else {
                                match node.submit_joint_configuration(members, node.millis()) {
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
            if node.state == NodeState::Leader
                && node.millis().saturating_sub(last_heartbeat_broadcast) >= heartbeat_interval_ms
            {
                node.broadcast_heartbeat(node.millis());
                last_heartbeat_broadcast = node.millis();
            }
            if !timer_armed {
                timer_armed = submit_detached_timer(LOOP_TICK_MS, 0, RAFT_TIMER_COOKIE) != u64::MAX;
            }
        }

        // --- Cluster-event publishing (best-effort) ---
        // The dns is the replicated event broker: conditions published here
        // (membership.{id} after a JOIN finalizes) are
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
            // Only the Raft leader announces admissions. Publishing a
            // synthetic single-node "established" event first used to leave
            // this path serialized behind a stale forwarded DNS call, which
            // prevented the real membership condition from ever being
            // announced. Membership itself is the authoritative condition.
            if node.state == NodeState::Leader {
                for peer in node.cluster_configuration.all_members() {
                    if peer.id != node.me.id && !events.fired_members.contains(&peer.id) {
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
                if result > 0
                    && let Some(id) = name.strip_prefix(b"event:membership:")
                {
                    events.fired_members.push(core::str::from_utf8(id).unwrap_or("").to_string());
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
            28,
            match node.state {
                NodeState::Follower => 1,
                NodeState::Candidate => 2,
                NodeState::Leader => 3,
            },
        );
        config::write::<u32>(32, node.cluster_configuration.all_members().len() as u32);
        config::write::<u32>(
            36,
            (u32::from(net_conn != 0))
                | (u32::from(frouter_conn != 0) << 1)
                | (u32::from(disco_conn != 0) << 2)
                | (u32::from(join_request_pending) << 3)
                | (u32::from(join_accepted) << 4)
                | (u32::from(node.joining) << 5),
        );
        config::write::<u32>(40, join_attempts);
        config::write::<u32>(44, join_requests_received);
        config::write::<u32>(48, join_replies_received);
        config::write::<u32>(52, node.millis().min(u32::MAX as u64) as u32);
        config::write::<u32>(56, mac_transport.peer_count().min(u32::MAX as usize) as u32);
        config::write::<u32>(60, mac_transport.pending_send_count().min(u32::MAX as usize) as u32);
        config::write::<u32>(64, mac_transport.outbound_count().min(u32::MAX as usize) as u32);
        for (index, count) in raft_tag_counts.iter().copied().enumerate() {
            config::write::<u32>(68 + index * 4, count);
        }
        config::write::<u32>(92, node.commit_index.min(u32::MAX as u64) as u32);
        config::write::<u32>(96, node.log_store.last_index().min(u32::MAX as u64) as u32);
        config::write::<u32>(100, node.log_store.last_term().min(u32::MAX as u64) as u32);
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
