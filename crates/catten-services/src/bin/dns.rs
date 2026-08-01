//! Distributed name service (`dns`) — a Raft-replicated `name -> node` catalog.
//!
//! One replica runs per node. Each replica:
//! - derives its persistent node identity from the NIC MAC + cluster mnemonic ([`NodeIdentity`])
//!   and waits for the kernel's boot-done marker,
//! - discovers its peers through the cluster discovery service (`disco`),
//! - runs a [`RaftNode`] whose transport carries peer RPCs over the reliable message layer
//!   ([`RelmsgRaftTransport`]), and
//! - serves registrations (proposed to the cluster, then registered with the node-local name
//!   service) and lookups (answered from the replicated catalog: local names resolve to the local
//!   name service, remote names report the hosting node).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    boxed::Box,
    collections::{
        BTreeMap,
        VecDeque,
    },
    string::ToString,
    sync::Arc,
    vec::Vec,
};

use catten_graft::{
    membership::ClusterConfiguration,
    node::RaftNode,
    state_machine::{
        QueryableStateMachine,
        StateMachine,
    },
    types::{
        NodeState,
        Peer,
    },
    wire::{
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
    name_catalog::{
        CatalogEntry,
        NameCatalog,
        decode_query_result,
        encode_activate,
        encode_register,
        encode_unregister_generation,
    },
    net,
    node_identity::NodeIdentity,
    ns,
    relmsg,
    relmsg_transport::{
        InboundRpc,
        RelmsgRaftTransport,
        TAG_APPEND_RESPONSE,
        TAG_SNAPSHOT_RESPONSE,
        TAG_VOTE_RESPONSE,
    },
    wait_for_boot_done,
    wait_reply,
};
use catten_syscall::{
    IpcRights,
    close as completion_close,
    cq_read,
    cq_wait_timeout,
    ipc_close,
    ipc_connection_watch_closed,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_connection,
    ipc_reply_move,
    ipc_reply_poll_with_memory,
    ipc_reply_wait_with_memory,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_scalar_call_move,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map,
    memory_unmap,
    poll as completion_poll,
    submit_detached_timer,
    thread_exit,
};
use charlotte_protocol_msg::unpack_address_and_len;

const LOOP_TICK_MS: u64 = 25;
const RAFT_TIMER_COOKIE: u64 = 0x444e_535f_5449_434b;
const REPLY_SPINS: u64 = u64::MAX;
const RX_SCRATCH: usize = 0x0000_0000_0090_0000;
const LIST_SCRATCH: usize = 0x0000_0000_0090_1000;
const CATALOG_SCRATCH: usize = 0x0000_0000_0090_2000;

const CLUSTER_KEY: u64 = manifest_key(b"cluster");
const EXPECTED_PEERS_KEY: u64 = manifest_key(b"peers");
const MEMBER_KEY: u64 = manifest_key(b"member");
const ELECTION_KEY: u64 = manifest_key(b"elect-ms");
const REMOTE_CALL_TIMEOUT_MS: u64 = 5_000;
const MAX_IN_FLIGHT_CALLS: usize = 64;
const DEDUP_WINDOW: usize = 128;

struct InFlightCall {
    call_id: u64,
    expected_peer: alloc::string::String,
    expected_generation: u64,
    reply: u64,
    deadline: u64,
}

struct CompletedCall {
    caller: Vec<u8>,
    session: u64,
    call_id: u64,
    result: i64,
    peer: alloc::string::String,
    settled_after_ack: u64,
}

enum PendingQueryKind {
    Lookup {
        reply: u64,
        name: Vec<u8>,
    },
    Call {
        reply: u64,
        name: Vec<u8>,
        opcode: u32,
        arg: i64,
    },
}

struct PendingQuery {
    query_id: u64,
    expected_leader: alloc::string::String,
    deadline: u64,
    kind: PendingQueryKind,
}

enum PendingRegistration {
    Prepare {
        log_index: u64,
        reply: u64,
        name: Vec<u8>,
        connection: u64,
        existing_local_generation: u64,
    },
    Activate {
        log_index: u64,
        reply: u64,
        name: Vec<u8>,
        generation: u64,
        connection: u64,
        local_generation: u64,
    },
    Unregister {
        log_index: u64,
        reply: u64,
        name: Vec<u8>,
        expected_generation: u64,
        local_generation: u64,
        automatic_term: Option<u64>,
    },
}

struct LocalPublication {
    name: Vec<u8>,
    generation: u64,
    local_generation: u64,
    connection: u64,
    close_watch: u64,
    endpoint_closed: bool,
    local_cleanup_submitted: bool,
    next_unregister_attempt: u64,
}

const AUTO_UNREGISTER_RETRY_MS: u64 = 1_000;

fn reply_lookup(
    ns_conn: u64,
    reply: u64,
    name: &[u8],
    entry: Option<CatalogEntry>,
    local_node: &[u8],
) {
    if reply == 0 {
        return;
    }
    let Some(entry) = entry else {
        ipc_reply(reply, dns::ERR_NOT_FOUND);
        return;
    };
    if entry.node == local_node {
        let lookup = ipc_scalar_call(ns_conn, ns::OP_TRY_LOOKUP, catten_services::name(name));
        let (generation, connection) = if lookup != 0 {
            unsafe { wait_reply(lookup, REPLY_SPINS) }
        } else {
            (0, 0)
        };
        if generation >= 1 && connection != 0 {
            ipc_reply_connection(
                reply,
                connection,
                IpcRights::SEND | IpcRights::CALL,
                dns::RESULT_LOCAL,
            );
        } else {
            ipc_reply(reply, dns::RESULT_LOCAL);
        }
        return;
    }

    let cap = memory_alloc(1);
    if cap != 0 && memory_map(cap, LIST_SCRATCH, true) == 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(
                entry.node.as_ptr(),
                LIST_SCRATCH as *mut u8,
                entry.node.len(),
            );
        }
        memory_unmap(cap);
        ipc_reply_move(reply, cap, dns::RESULT_REMOTE);
    } else {
        if cap != 0 {
            memory_close(cap);
        }
        ipc_reply(reply, dns::ERR_NOT_FOUND);
    }
}

fn linearizable_entry(node: &RaftNode, name: &[u8]) -> Result<Option<CatalogEntry>, i64> {
    node.handle_client_query(name.to_vec())
        .map(|bytes| decode_query_result(&bytes))
        .map_err(|_| dns::ERR_NOT_LEADER)
}

fn persistent_namespace(cluster_id: &[u8], node_id: &[u8]) -> u64 {
    // Stable FNV-1a over the cluster/node tuple. This selects an object-store
    // namespace; it is not used as a security boundary.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in
        cluster_id.iter().copied().chain(core::iter::once(0xff)).chain(node_id.iter().copied())
    {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Boxes an `Arc<NameCatalog>` as a `StateMachine` so the Raft node and the
/// service share one catalog.
struct CatalogMachine(Arc<NameCatalog>);

impl StateMachine for CatalogMachine {
    fn apply(&self, term: u64, command: &[u8]) {
        self.0.apply(term, command);
    }

    fn apply_with_result(&self, term: u64, command: &[u8]) -> Vec<u8> {
        self.0.apply_with_result(term, command)
    }

    fn snapshot(&self) -> Vec<u8> {
        self.0.snapshot()
    }

    fn restore(&self, snapshot_data: &[u8]) {
        self.0.restore(snapshot_data);
    }

    fn as_queryable(&self) -> Option<&dyn QueryableStateMachine> {
        Some(self.0.as_ref())
    }
}

fn fatal(stage: u64) -> ! {
    catten_syscall::el0_log(0x444e_5300, stage);
    unsafe { thread_exit() }
}

/// Query the disco service for the current discovered peer list
/// `(mac, node_id)`.
fn query_disco_peers(disco_conn: u64) -> Vec<([u8; 6], Vec<u8>)> {
    let call = ipc_scalar_call(disco_conn, disco::OP_LIST_PEERS, 0);
    if call == 0 {
        return Vec::new();
    }
    let (status, result, _returned_connection, memory) = ipc_reply_wait_with_memory(call);
    ipc_close(call);
    if status != 0 || memory == 0 {
        if memory != 0 {
            memory_close(memory);
        }
        return Vec::new();
    }
    let len = result as usize;
    if memory_map(memory, LIST_SCRATCH, false) != 0 {
        memory_close(memory);
        return Vec::new();
    }
    let mut buf = Vec::with_capacity(len);
    unsafe {
        let src = LIST_SCRATCH as *const u8;
        for i in 0..len {
            buf.push(core::ptr::read_volatile(src.add(i)));
        }
        memory_unmap(memory);
    }
    memory_close(memory);
    charlotte_protocol_disco::parse_peer_list(&buf)
}

fn drive_inbound(
    node: &mut RaftNode,
    transport: &RelmsgRaftTransport,
    source_mac: [u8; 6],
    inbound: InboundRpc,
    millis: u64,
) {
    match inbound {
        InboundRpc::VoteRequest(request) => {
            let response = node.handle_vote_request(request, millis);
            if let Ok(payload) = encode_vote_response(&response) {
                transport.send_response(source_mac, TAG_VOTE_RESPONSE, payload);
            }
        }
        InboundRpc::AppendEntries(request) => {
            let response = node.handle_append_entries(request, millis);
            if let Ok(payload) = encode_append_response(&response) {
                transport.send_response(source_mac, TAG_APPEND_RESPONSE, payload);
            }
        }
        InboundRpc::InstallSnapshot(request) => {
            let response = node.handle_install_snapshot(request, millis);
            if let Ok(payload) = encode_snapshot_response(&response) {
                transport.send_response(source_mac, TAG_SNAPSHOT_RESPONSE, payload);
            }
        }
    }
}

/// Invoke a service registered with the node-local name service, returning
/// its scalar result.
fn invoke_local(ns_conn: u64, name: &[u8], opcode: u32, arg: i64) -> i64 {
    let lookup = ipc_scalar_call(ns_conn, ns::OP_TRY_LOOKUP, catten_services::name(name));
    if lookup == 0 {
        return dns::ERR_NOT_FOUND;
    }
    let (generation, conn) = unsafe { wait_reply(lookup, REPLY_SPINS) };
    if generation < 1 || conn == 0 {
        return dns::ERR_NOT_FOUND;
    }
    let call = ipc_scalar_call(conn, opcode, arg as u64);
    if call == 0 {
        return dns::ERR_NOT_FOUND;
    }
    let (result, _) = unsafe { wait_reply(call, REPLY_SPINS) };
    result
}

/// Read the `[opcode:u32 LE][arg:i64 LE]` request from an `OP_CALL` memory
/// object (consuming it).
fn read_call_request(message: &catten_syscall::IpcMessage) -> (u32, i64) {
    if message.memory == 0 {
        return (0, 0);
    }
    if memory_map(message.memory, LIST_SCRATCH, false) != 0 {
        memory_close(message.memory);
        return (0, 0);
    }
    let opcode = unsafe { core::ptr::read_volatile(LIST_SCRATCH as *const u32) };
    let arg = unsafe { core::ptr::read_volatile((LIST_SCRATCH + 4) as *const i64) };
    memory_unmap(message.memory);
    memory_close(message.memory);
    (opcode, arg)
}

fn read_generation(message: &catten_syscall::IpcMessage) -> Option<u64> {
    if message.memory == 0 {
        return None;
    }
    if memory_map(message.memory, LIST_SCRATCH, false) != 0 {
        memory_close(message.memory);
        return None;
    }
    let generation = unsafe { core::ptr::read_volatile(LIST_SCRATCH as *const u64) };
    memory_unmap(message.memory);
    memory_close(message.memory);
    Some(generation)
}

fn local_generation(ns_conn: u64, name: &[u8]) -> u64 {
    let lookup = ipc_scalar_call(ns_conn, ns::OP_TRY_LOOKUP, catten_services::name(name));
    if lookup == 0 {
        return 0;
    }
    let (generation, connection) = unsafe { wait_reply(lookup, REPLY_SPINS) };
    if connection != 0 {
        ipc_close(connection);
    }
    generation.max(0) as u64
}

fn submit_unregister_local_generation(ns_conn: u64, name: &[u8], generation: u64) -> u64 {
    let memory = memory_alloc(1);
    if memory == 0 || memory_map(memory, LIST_SCRATCH, true) != 0 {
        if memory != 0 {
            memory_close(memory);
        }
        return 0;
    }
    unsafe {
        core::ptr::write_volatile(LIST_SCRATCH as *mut u64, generation);
    }
    memory_unmap(memory);
    let call = ipc_scalar_call_move(
        ns_conn,
        ns::OP_UNREGISTER_GENERATION,
        catten_services::name(name),
        memory,
    );
    if call == 0 {
        memory_close(memory);
        return 0;
    }
    call
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(0, 1);
    let mnemonic: Vec<u8> = match ctx.manifest_value(CLUSTER_KEY) {
        Some(ManifestValue::Bytes(raw)) if !raw.is_empty() => raw.to_vec(),
        _ => b"default".to_vec(),
    };
    let expected_peers = match ctx.manifest_value(EXPECTED_PEERS_KEY) {
        Some(ManifestValue::Unsigned(value)) => value,
        _ => 1,
    };
    let election_timeout_ms = match ctx.manifest_value(ELECTION_KEY) {
        Some(ManifestValue::Unsigned(value)) => value,
        _ => 300,
    };

    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => fatal(1),
    };
    config::write::<u32>(0, 2);

    // MAC and persisted node identity.
    let net_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, net::NAME);
    if net_lookup == 0 {
        fatal(2);
    }
    let (net_generation, net_conn) = unsafe { wait_reply(net_lookup, REPLY_SPINS) };
    if net_generation < 1 || net_conn == 0 {
        fatal(3);
    }
    let status_call = ipc_scalar_call(net_conn, net::OP_STATUS, 0);
    if status_call == 0 {
        fatal(4);
    }
    let (status, _) = unsafe { wait_reply(status_call, REPLY_SPINS) };
    let (link, local_mac) = charlotte_protocol_net::decode_status(status);
    if link == 0 {
        fatal(5);
    }
    let identity = match NodeIdentity::load_or_create(ns_conn, &mnemonic, Some(local_mac)) {
        Some(identity) => identity,
        None => fatal(6),
    };
    let node_name = identity.name;
    let node_name_str = core::str::from_utf8(&node_name).unwrap_or("node").to_string();
    let mut configured_members: Vec<alloc::string::String> = ctx
        .manifest()
        .filter(|entry| entry.key == MEMBER_KEY)
        .filter_map(|entry| match entry.value {
            ManifestValue::Bytes(value) => core::str::from_utf8(value).ok(),
            _ => None,
        })
        .filter(|member| !member.is_empty())
        .map(ToString::to_string)
        .collect();
    configured_members.sort();
    configured_members.dedup();
    if configured_members.is_empty() {
        if expected_peers != 1 {
            // Discovery identifies routes, but it must not independently
            // grant voting authority. A multi-voter cluster requires the
            // exact authoritative member identities in its launch manifest.
            fatal(18);
        }
        configured_members.push(node_name_str.clone());
    }
    if configured_members.len() as u64 != expected_peers
        || !configured_members.iter().any(|member| member == &node_name_str)
    {
        fatal(19);
    }
    config::write::<u32>(0, 3);

    // Wait for the boot storm to settle before joining the cluster.
    if !wait_for_boot_done(ns_conn) {
        fatal(7);
    }
    config::write::<u32>(0, 4);

    let relmsg_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, relmsg::NAME);
    if relmsg_lookup == 0 {
        fatal(8);
    }
    let (relmsg_generation, relmsg_conn) = unsafe { wait_reply(relmsg_lookup, REPLY_SPINS) };
    if relmsg_generation < 1 || relmsg_conn == 0 {
        fatal(9);
    }
    let disco_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, disco::NAME);
    if disco_lookup == 0 {
        fatal(10);
    }
    let (disco_generation, disco_conn) = unsafe { wait_reply(disco_lookup, REPLY_SPINS) };
    if disco_generation < 1 || disco_conn == 0 {
        fatal(11);
    }
    config::write::<u32>(0, 5);

    // The dns endpoint: services register and look up through this service.
    let endpoint = ipc_endpoint_create(dns::INTERFACE, dns::VERSION, 16);
    if endpoint == 0 {
        fatal(12);
    }
    if ipc_endpoint_bind_cq(endpoint, 0) != 0 {
        fatal(13);
    }
    let register = ipc_scalar_call_connection(
        ns_conn,
        ns::OP_REGISTER,
        dns::NAME,
        endpoint,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if register == 0 {
        fatal(14);
    }
    let (generation, _) = unsafe { wait_reply(register, REPLY_SPINS) };
    if generation < 1 {
        fatal(15);
    }
    let dns_session = generation as u64;
    config::write::<u32>(0, 6);

    // Resolve the configured voter identities to transient MAC routes. The
    // launch manifest grants voting authority; discovery never does.
    let transport = Arc::new(RelmsgRaftTransport::new(relmsg_conn));
    let mut discovered: BTreeMap<alloc::string::String, [u8; 6]> = BTreeMap::new();
    let mut discovery_rounds: u64 = 0;
    loop {
        discovered.clear();
        for (mac, peer_node_id) in query_disco_peers(disco_conn) {
            let Ok(peer_name) = core::str::from_utf8(&peer_node_id) else {
                continue;
            };
            if peer_name != node_name_str
                && configured_members.iter().any(|member| member == peer_name)
            {
                discovered.insert(peer_name.to_string(), mac);
            }
        }
        let all_remote_members_visible = configured_members
            .iter()
            .filter(|member| *member != &node_name_str)
            .all(|member| discovered.contains_key(member));
        if all_remote_members_visible {
            break;
        }
        if discovery_rounds >= 2400 {
            fatal(20);
        }
        if !all_remote_members_visible {
            catten_services::sleep_ms(50);
            discovery_rounds += 1;
        }
    }
    config::write::<u32>(0, 7);

    let mut peers = Vec::new();
    let me = Peer::voter(node_name_str.clone(), 0);
    peers.push(me.clone());
    for peer_name in &configured_members {
        if peer_name == &node_name_str {
            continue;
        }
        let Some(mac) = discovered.get(peer_name).copied() else {
            fatal(21);
        };
        transport.add_peer(peer_name, mac);
        peers.push(Peer::voter(peer_name.clone(), 0));
    }
    config::write::<u32>(8, peers.len() as u32);

    let catalog = NameCatalog::new();
    // A clustered voter must retain term, vote, log, and snapshot state.
    // Falling back to memory after advertising the same durable node identity
    // would permit a restarted replica to vote twice in one term.
    let namespace = persistent_namespace(&mnemonic, &node_name);
    let log_store = match DiskLogStore::new(ns_conn, namespace, true) {
        Some(store) => store,
        None => fatal(16),
    };
    let persistent_state = match DiskPersistentStateStore::new(ns_conn, namespace, true) {
        Some(store) => store,
        None => fatal(17),
    };
    let mut node = RaftNode::new(catten_graft::node::RaftNodeConfig {
        me,
        timeout_millis: election_timeout_ms,
        log_store: Box::new(log_store),
        persistent_state: Box::new(persistent_state),
        state_machine: Some(Box::new(CatalogMachine(catalog.clone()))),
        cluster_configuration: ClusterConfiguration::stable(peers),
        transport: transport.clone(),
        current_millis: 0,
        snapshot_min_entries: 0,
        snapshot_chunk_bytes: 1200,
    });
    config::write::<u32>(0, 8);

    let cq = ctx.completion_queue_layout();
    let mut recv_pending: u64 = 0;
    let mut served: u32 = 0;
    let mut remote_calls_served: u32 = 0;
    let mut remote_queries_served: u32 = 0;
    let mut pending_registers: Vec<PendingRegistration> = Vec::new();
    let mut in_flight_calls: Vec<InFlightCall> = Vec::new();
    let mut completed_calls: VecDeque<CompletedCall> = VecDeque::new();
    let mut next_reply_ordinal: BTreeMap<alloc::string::String, u64> = BTreeMap::new();
    let mut next_call_id: u64 = 1;
    let mut pending_queries: Vec<PendingQuery> = Vec::new();
    let mut pending_local_unregistrations: Vec<u64> = Vec::new();
    let mut local_publications: Vec<LocalPublication> = Vec::new();
    let mut next_query_id: u64 = 1;
    let mut timer_armed = submit_detached_timer(LOOP_TICK_MS, 0, RAFT_TIMER_COOKIE) != u64::MAX;

    loop {
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

        // --- Inbound Raft traffic over relmsg ---
        if recv_pending == 0 {
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
                            Some(catten_services::rcall::TAG_REQUEST) => {
                                // A remote invocation addressed to this node:
                                // execute it against the local name service and
                                // reply to the caller's MAC.
                                if let Some((
                                    session,
                                    call_id,
                                    caller,
                                    target,
                                    target_generation,
                                    opcode,
                                    arg,
                                )) = catten_services::rcall::decode_request(frame)
                                {
                                    let Some(source_peer) = transport.peer_id_for_mac(&source_mac)
                                    else {
                                        memory_unmap(memory);
                                        memory_close(memory);
                                        continue;
                                    };
                                    if source_peer.as_bytes() != caller {
                                        memory_unmap(memory);
                                        memory_close(memory);
                                        continue;
                                    }
                                    let cached_result = completed_calls
                                        .iter()
                                        .find(|completed| {
                                            completed.caller == caller
                                                && completed.session == session
                                                && completed.call_id == call_id
                                        })
                                        .map(|completed| completed.result);
                                    let reply_ordinal = next_reply_ordinal
                                        .entry(source_peer.clone())
                                        .or_insert_with(|| {
                                            transport.acknowledged_count_for(
                                                &source_peer,
                                                catten_services::rcall::TAG_REPLY,
                                            )
                                        });
                                    *reply_ordinal = reply_ordinal.saturating_add(1);

                                    if cached_result.is_none()
                                        && completed_calls.len() >= DEDUP_WINDOW
                                        && let Some(index) =
                                            completed_calls.iter().position(|completed| {
                                                transport.acknowledged_count_for(
                                                    &completed.peer,
                                                    catten_services::rcall::TAG_REPLY,
                                                ) >= completed.settled_after_ack
                                            })
                                    {
                                        completed_calls.remove(index);
                                    }
                                    let has_dedup_capacity = completed_calls.len() < DEDUP_WINDOW;
                                    let result = cached_result.unwrap_or_else(|| {
                                        if !has_dedup_capacity {
                                            return dns::ERR_BUSY;
                                        }
                                        match catalog.lookup(&target) {
                                            Some(owner)
                                                if owner.node == node_name
                                                    && owner.generation == target_generation =>
                                            {
                                                invoke_local(ns_conn, &target, opcode, arg)
                                            }
                                            Some(owner) if owner.node == node_name => {
                                                dns::ERR_STALE_GENERATION
                                            }
                                            _ => dns::ERR_NOT_FOUND,
                                        }
                                    });
                                    if cached_result.is_none() && has_dedup_capacity {
                                        completed_calls.push_back(CompletedCall {
                                            caller,
                                            session,
                                            call_id,
                                            result,
                                            peer: source_peer.clone(),
                                            settled_after_ack: *reply_ordinal,
                                        });
                                    }
                                    remote_calls_served = remote_calls_served.wrapping_add(1);
                                    config::write::<u32>(40, remote_calls_served);
                                    let reply = catten_services::rcall::encode_reply(
                                        session,
                                        call_id,
                                        target_generation,
                                        result,
                                    );
                                    transport.send_message(
                                        &source_peer,
                                        catten_services::rcall::TAG_REPLY,
                                        reply,
                                    );
                                }
                            }
                            Some(catten_services::rcall::TAG_REPLY) => {
                                // Complete the matching in-flight OP_CALL.
                                if let Some((session, call_id, target_generation, result)) =
                                    catten_services::rcall::decode_reply(frame)
                                    && let Some(index) = in_flight_calls.iter().position(|call| {
                                        call.call_id == call_id
                                            && session == dns_session
                                            && target_generation == call.expected_generation
                                            && transport
                                                .peer_id_for_mac(&source_mac)
                                                .is_some_and(|peer| peer == call.expected_peer)
                                    })
                                {
                                    let call = in_flight_calls.swap_remove(index);
                                    if call.reply != 0 {
                                        ipc_reply(call.reply, result);
                                    }
                                }
                            }
                            Some(catten_services::rquery::TAG_REQUEST) => {
                                if let Some((session, query_id, caller, name)) =
                                    catten_services::rquery::decode_request(frame)
                                    && let Some(source_peer) =
                                        transport.peer_id_for_mac(&source_mac)
                                    && source_peer.as_bytes() == caller
                                {
                                    remote_queries_served = remote_queries_served.wrapping_add(1);
                                    config::write::<u32>(44, remote_queries_served);
                                    let (status, entry) =
                                        match node.handle_client_query(name.clone()) {
                                            Ok(bytes) => (0, decode_query_result(&bytes)),
                                            Err(_) => (dns::ERR_NOT_LEADER, None),
                                        };
                                    let reply = catten_services::rquery::encode_reply(
                                        session,
                                        query_id,
                                        status,
                                        entry.as_ref().map_or(0, |value| value.generation),
                                        entry
                                            .as_ref()
                                            .map_or(&[][..], |value| value.node.as_slice()),
                                    );
                                    transport.send_message(
                                        &source_peer,
                                        catten_services::rquery::TAG_REPLY,
                                        reply,
                                    );
                                }
                            }
                            Some(catten_services::rquery::TAG_REPLY) => {
                                if let Some((session, query_id, status, generation, owner)) =
                                    catten_services::rquery::decode_reply(frame)
                                    && session == dns_session
                                    && let Some(source_peer) =
                                        transport.peer_id_for_mac(&source_mac)
                                    && let Some(index) = pending_queries.iter().position(|query| {
                                        query.query_id == query_id
                                            && query.expected_leader == source_peer
                                    })
                                {
                                    let query = pending_queries.swap_remove(index);
                                    let entry =
                                        (status == 0 && generation != 0 && !owner.is_empty())
                                            .then_some(CatalogEntry {
                                                node: owner,
                                                generation,
                                                active: true,
                                            });
                                    match query.kind {
                                        PendingQueryKind::Lookup {
                                            reply,
                                            name,
                                        } => {
                                            if status == 0 {
                                                reply_lookup(
                                                    ns_conn, reply, &name, entry, &node_name,
                                                );
                                            } else if reply != 0 {
                                                ipc_reply(reply, status);
                                            }
                                        }
                                        PendingQueryKind::Call {
                                            reply,
                                            name,
                                            opcode,
                                            arg,
                                        } => {
                                            let result = if status != 0 {
                                                Some(status)
                                            } else if let Some(entry) = entry {
                                                if entry.node == node_name {
                                                    Some(invoke_local(ns_conn, &name, opcode, arg))
                                                } else if in_flight_calls.len()
                                                    >= MAX_IN_FLIGHT_CALLS
                                                {
                                                    Some(dns::ERR_BUSY)
                                                } else {
                                                    let owner_str =
                                                        core::str::from_utf8(&entry.node)
                                                            .unwrap_or("")
                                                            .to_string();
                                                    if !transport.has_peer(&owner_str) {
                                                        Some(dns::ERR_NOT_FOUND)
                                                    } else {
                                                        let call_id = next_call_id;
                                                        next_call_id =
                                                            next_call_id.wrapping_add(1).max(1);
                                                        in_flight_calls.push(InFlightCall {
                                                            call_id,
                                                            expected_peer: owner_str.clone(),
                                                            expected_generation: entry.generation,
                                                            reply,
                                                            deadline: node.millis().saturating_add(
                                                                REMOTE_CALL_TIMEOUT_MS,
                                                            ),
                                                        });
                                                        let request =
                                                            catten_services::rcall::encode_request(
                                                                dns_session,
                                                                call_id,
                                                                &node_name,
                                                                &name,
                                                                entry.generation,
                                                                opcode,
                                                                arg,
                                                            );
                                                        transport.send_message(
                                                            &owner_str,
                                                            catten_services::rcall::TAG_REQUEST,
                                                            request,
                                                        );
                                                        None
                                                    }
                                                }
                                            } else {
                                                Some(dns::ERR_NOT_FOUND)
                                            };
                                            if let Some(result) = result
                                                && reply != 0
                                            {
                                                ipc_reply(reply, result);
                                            }
                                        }
                                    }
                                }
                            }
                            Some(catten_services::runregister::TAG_REQUEST) => {
                                if let Some((owner, name, generation)) =
                                    catten_services::runregister::decode_request(frame)
                                    && node.state == NodeState::Leader
                                    && transport
                                        .peer_id_for_mac(&source_mac)
                                        .is_some_and(|peer| peer.as_bytes() == owner)
                                    && catalog.lookup(&name).is_some_and(|entry| {
                                        entry.active
                                            && entry.node == owner
                                            && entry.generation == generation
                                    })
                                    && !pending_registers.iter().any(|pending| matches!(
                                        pending,
                                        PendingRegistration::Unregister {
                                            name: pending_name,
                                            expected_generation,
                                            automatic_term: Some(_),
                                            ..
                                        } if pending_name == &name && *expected_generation == generation
                                    ))
                                    && let Ok(log_index) = node.submit_command(
                                        encode_unregister_generation(&name, &owner, generation),
                                        node.millis(),
                                    )
                                {
                                    pending_registers.push(PendingRegistration::Unregister {
                                        log_index,
                                        reply: 0,
                                        name,
                                        expected_generation: generation,
                                        local_generation: 0,
                                        automatic_term: Some(node.current_term),
                                    });
                                }
                            }
                            _ => {
                                if let Some(inbound) = transport.decode_inbound(&source_mac, frame)
                                {
                                    let millis = node.millis();
                                    drive_inbound(
                                        &mut node, &transport, source_mac, inbound, millis,
                                    );
                                }
                            }
                        }
                        memory_unmap(memory);
                    }
                    memory_close(memory);
                }
            }
        }

        transport.drain_outbound();
        transport.reap_acks();
        config::write::<u32>(
            36,
            transport.acknowledged_count(catten_services::rcall::TAG_REPLY).min(u32::MAX as u64)
                as u32,
        );

        let completed = node.poll_transport(node.millis());
        if completed > 0 {
            config::write::<u32>(12, completed as u32);
        }

        // --- Local endpoint ops (register / lookup / status) ---
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
            config::write::<u32>(16, served);

            match message.opcode {
                dns::OP_REGISTER => {
                    let name = packed_name(message.arg0);
                    let result = if name.is_empty() {
                        dns::ERR_TOO_LARGE
                    } else {
                        // Submit once; the reactor completes the reply once the
                        // entry has replicated (see pending_registers below).
                        match node.submit_command(encode_register(&name, &node_name), node.millis())
                        {
                            Ok(index) => {
                                let (connection, existing_local_generation) =
                                    if message.connection != 0 {
                                        (message.connection, 0)
                                    } else {
                                        let lookup = ipc_scalar_call(
                                            ns_conn,
                                            ns::OP_TRY_LOOKUP,
                                            catten_services::name(&name),
                                        );
                                        if lookup == 0 {
                                            (0, 0)
                                        } else {
                                            let (generation, connection) =
                                                unsafe { wait_reply(lookup, REPLY_SPINS) };
                                            if generation >= 1 && connection != 0 {
                                                (connection, generation as u64)
                                            } else {
                                                (0, 0)
                                            }
                                        }
                                    };
                                pending_registers.push(PendingRegistration::Prepare {
                                    log_index: index,
                                    reply: message.reply,
                                    name,
                                    connection,
                                    existing_local_generation,
                                });
                                continue;
                            }
                            Err(code) => code,
                        }
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_UNREGISTER => {
                    let name = packed_name(message.arg0);
                    let expected_generation = read_generation(&message);
                    let result = if name.is_empty() || expected_generation.is_none() {
                        dns::ERR_TOO_LARGE
                    } else if node.state != NodeState::Leader {
                        dns::ERR_NOT_LEADER
                    } else {
                        let expected_generation = expected_generation.unwrap_or(0);
                        let matches_active_owner = catalog.lookup(&name).is_some_and(|entry| {
                            entry.node == node_name && entry.generation == expected_generation
                        });
                        if !matches_active_owner {
                            if message.reply != 0 {
                                ipc_reply(message.reply, dns::ERR_STALE_GENERATION);
                            }
                            continue;
                        }
                        let local_generation = local_generation(ns_conn, &name);
                        match node.submit_command(
                            encode_unregister_generation(&name, &node_name, expected_generation),
                            node.millis(),
                        ) {
                            Ok(log_index) => {
                                pending_registers.push(PendingRegistration::Unregister {
                                    log_index,
                                    reply: message.reply,
                                    name,
                                    expected_generation,
                                    local_generation,
                                    automatic_term: None,
                                });
                                continue;
                            }
                            Err(code) => code,
                        }
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_LOOKUP => {
                    let name = packed_name(message.arg0);
                    let result = if name.is_empty() {
                        dns::ERR_TOO_LARGE
                    } else if node.state == NodeState::Leader {
                        match linearizable_entry(&node, &name) {
                            Ok(entry) => {
                                reply_lookup(ns_conn, message.reply, &name, entry, &node_name);
                                continue;
                            }
                            Err(code) => code,
                        }
                    } else {
                        let Some(leader) = node.known_leader_id.clone() else {
                            if message.reply != 0 {
                                ipc_reply(message.reply, dns::ERR_NOT_LEADER);
                            }
                            continue;
                        };
                        if pending_queries.len() >= MAX_IN_FLIGHT_CALLS
                            || !transport.has_peer(&leader)
                        {
                            dns::ERR_BUSY
                        } else {
                            let query_id = next_query_id;
                            next_query_id = next_query_id.wrapping_add(1).max(1);
                            pending_queries.push(PendingQuery {
                                query_id,
                                expected_leader: leader.clone(),
                                deadline: node.millis().saturating_add(REMOTE_CALL_TIMEOUT_MS),
                                kind: PendingQueryKind::Lookup {
                                    reply: message.reply,
                                    name: name.clone(),
                                },
                            });
                            let request = catten_services::rquery::encode_request(
                                dns_session,
                                query_id,
                                &node_name,
                                &name,
                            );
                            transport.send_message(
                                &leader,
                                catten_services::rquery::TAG_REQUEST,
                                request,
                            );
                            continue;
                        }
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_STATUS => {
                    let state = match node.state {
                        NodeState::Follower => 1,
                        NodeState::Candidate => 2,
                        NodeState::Leader => 3,
                    };
                    let result = (state as i64)
                        | ((node.current_term as i64) << 8)
                        | ((catalog.registered_count() as i64) << 32);
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_CATALOG => {
                    // Dump the replicated name -> node catalog into a moved
                    // page: [count:u32][name_len:u8 name node_len:u8 node
                    // generation:u64]*.
                    let cap = memory_alloc(1);
                    if cap == 0 {
                        if message.reply != 0 {
                            ipc_reply(message.reply, dns::ERR_BAD_OPCODE);
                        }
                        continue;
                    }
                    if memory_map(cap, CATALOG_SCRATCH, true) != 0 {
                        memory_close(cap);
                        if message.reply != 0 {
                            ipc_reply(message.reply, dns::ERR_BAD_OPCODE);
                        }
                        continue;
                    }
                    let entries = catalog.entries();
                    let mut length = 4usize;
                    unsafe {
                        core::ptr::write_volatile(
                            CATALOG_SCRATCH as *mut u32,
                            entries.len() as u32,
                        );
                    }
                    for (name, entry) in entries.iter() {
                        let name_len = name.len().min(255);
                        let node_len = entry.node.len().min(255);
                        if length + 2 + name_len + node_len + 8 > 4096 {
                            break;
                        }
                        unsafe {
                            core::ptr::write_volatile(
                                (CATALOG_SCRATCH + length) as *mut u8,
                                name_len as u8,
                            );
                            core::ptr::copy_nonoverlapping(
                                name.as_ptr(),
                                (CATALOG_SCRATCH + length + 1) as *mut u8,
                                name_len,
                            );
                            core::ptr::write_volatile(
                                (CATALOG_SCRATCH + length + 1 + name_len) as *mut u8,
                                node_len as u8,
                            );
                            core::ptr::copy_nonoverlapping(
                                entry.node.as_ptr(),
                                (CATALOG_SCRATCH + length + 2 + name_len) as *mut u8,
                                node_len,
                            );
                            core::ptr::copy_nonoverlapping(
                                entry.generation.to_le_bytes().as_ptr(),
                                (CATALOG_SCRATCH + length + 2 + name_len + node_len) as *mut u8,
                                8,
                            );
                        }
                        length += 2 + name_len + node_len + 8;
                    }
                    memory_unmap(cap);
                    if message.reply != 0 {
                        ipc_reply_move(message.reply, cap, length as i64);
                    } else {
                        memory_close(cap);
                    }
                }

                dns::OP_CALL => {
                    let name = packed_name(message.arg0);
                    let (opcode, arg) = read_call_request(&message);
                    let result = if name.is_empty() {
                        dns::ERR_TOO_LARGE
                    } else if node.state != NodeState::Leader {
                        let Some(leader) = node.known_leader_id.clone() else {
                            if message.reply != 0 {
                                ipc_reply(message.reply, dns::ERR_NOT_LEADER);
                            }
                            continue;
                        };
                        if pending_queries.len() >= MAX_IN_FLIGHT_CALLS
                            || !transport.has_peer(&leader)
                        {
                            dns::ERR_BUSY
                        } else {
                            let query_id = next_query_id;
                            next_query_id = next_query_id.wrapping_add(1).max(1);
                            pending_queries.push(PendingQuery {
                                query_id,
                                expected_leader: leader.clone(),
                                deadline: node.millis().saturating_add(REMOTE_CALL_TIMEOUT_MS),
                                kind: PendingQueryKind::Call {
                                    reply: message.reply,
                                    name: name.clone(),
                                    opcode,
                                    arg,
                                },
                            });
                            let request = catten_services::rquery::encode_request(
                                dns_session,
                                query_id,
                                &node_name,
                                &name,
                            );
                            transport.send_message(
                                &leader,
                                catten_services::rquery::TAG_REQUEST,
                                request,
                            );
                            continue;
                        }
                    } else {
                        match linearizable_entry(&node, &name) {
                            Ok(Some(owner)) if owner.node == node_name => {
                                invoke_local(ns_conn, &name, opcode, arg)
                            }
                            Ok(Some(owner)) => {
                                // Remote: relay to the hosting node's dns over
                                // the reliable message layer.
                                let owner_str =
                                    core::str::from_utf8(&owner.node).unwrap_or("").to_string();
                                if let Some(_mac) = transport.mac_for_peer(&owner_str) {
                                    if in_flight_calls.len() >= MAX_IN_FLIGHT_CALLS {
                                        dns::ERR_BUSY
                                    } else {
                                        let call_id = next_call_id;
                                        next_call_id = next_call_id.wrapping_add(1).max(1);
                                        in_flight_calls.push(InFlightCall {
                                            call_id,
                                            expected_peer: owner_str.clone(),
                                            expected_generation: owner.generation,
                                            reply: message.reply,
                                            deadline: node
                                                .millis()
                                                .saturating_add(REMOTE_CALL_TIMEOUT_MS),
                                        });
                                        let frame = catten_services::rcall::encode_request(
                                            dns_session,
                                            call_id,
                                            &node_name,
                                            &name,
                                            owner.generation,
                                            opcode,
                                            arg,
                                        );
                                        transport.send_message(
                                            &owner_str,
                                            catten_services::rcall::TAG_REQUEST,
                                            frame,
                                        );
                                        continue; // reply completes when the remote REPLY arrives
                                    }
                                } else {
                                    dns::ERR_NOT_FOUND
                                }
                            }
                            Ok(None) => dns::ERR_NOT_FOUND,
                            Err(code) => code,
                        }
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_SHUTDOWN => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, 0);
                    }
                    unsafe { thread_exit() };
                }

                _ => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_BAD_OPCODE);
                    }
                }
            }
        }

        // --- Complete deferred registers once committed ---
        let mut index = 0;
        while index < pending_registers.len() {
            if matches!(
                &pending_registers[index],
                PendingRegistration::Unregister {
                    automatic_term: Some(term),
                    ..
                } if *term != node.current_term
            ) {
                pending_registers.swap_remove(index);
                continue;
            }
            let log_index = match &pending_registers[index] {
                PendingRegistration::Prepare {
                    log_index,
                    ..
                }
                | PendingRegistration::Activate {
                    log_index,
                    ..
                }
                | PendingRegistration::Unregister {
                    log_index,
                    ..
                } => *log_index,
            };
            if !node.is_committed(log_index) {
                index += 1;
                continue;
            }
            match pending_registers.swap_remove(index) {
                PendingRegistration::Prepare {
                    reply,
                    name,
                    connection,
                    existing_local_generation,
                    ..
                } => {
                    let generation = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(u64::from_le_bytes)
                        .unwrap_or(0);
                    let local_generation = if generation == 0 {
                        None
                    } else if existing_local_generation != 0 {
                        Some(existing_local_generation)
                    } else if connection == 0 {
                        Some(0)
                    } else {
                        let local_reg = ipc_scalar_call_connection(
                            ns_conn,
                            ns::OP_REGISTER,
                            catten_services::name(&name),
                            connection,
                            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
                        );
                        if local_reg == 0 {
                            None
                        } else {
                            let (local_generation, _) =
                                unsafe { wait_reply(local_reg, REPLY_SPINS) };
                            (local_generation >= 1).then_some(local_generation as u64)
                        }
                    };
                    let Some(local_generation) = local_generation else {
                        if connection != 0 {
                            ipc_close(connection);
                        }
                        if reply != 0 {
                            ipc_reply(reply, dns::ERR_TOO_LARGE);
                        }
                        continue;
                    };
                    match node.submit_command(encode_activate(&name, generation), node.millis()) {
                        Ok(activate_index) => {
                            pending_registers.push(PendingRegistration::Activate {
                                log_index: activate_index,
                                reply,
                                name,
                                generation,
                                connection,
                                local_generation,
                            });
                        }
                        Err(code) => {
                            if local_generation != 0 {
                                let unregister = submit_unregister_local_generation(
                                    ns_conn,
                                    &name,
                                    local_generation,
                                );
                                if unregister != 0 {
                                    pending_local_unregistrations.push(unregister);
                                }
                            }
                            if connection != 0 {
                                ipc_close(connection);
                            }
                            if reply != 0 {
                                ipc_reply(reply, code);
                            }
                        }
                    }
                }
                PendingRegistration::Activate {
                    reply,
                    name,
                    generation,
                    connection,
                    local_generation,
                    ..
                } => {
                    let activated = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(u64::from_le_bytes)
                        == Some(generation);
                    if activated && connection != 0 {
                        let close_watch = ipc_connection_watch_closed(connection);
                        config::write::<u32>(
                            32,
                            if close_watch == u64::MAX {
                                u32::MAX
                            } else {
                                1
                            },
                        );
                        local_publications.push(LocalPublication {
                            name: name.clone(),
                            generation,
                            local_generation,
                            connection,
                            close_watch,
                            endpoint_closed: false,
                            local_cleanup_submitted: false,
                            next_unregister_attempt: 0,
                        });
                    } else if connection != 0 {
                        ipc_close(connection);
                    }
                    if reply != 0 {
                        ipc_reply(
                            reply,
                            if activated {
                                generation as i64
                            } else {
                                dns::ERR_NOT_FOUND
                            },
                        );
                    }
                }
                PendingRegistration::Unregister {
                    reply,
                    name,
                    expected_generation,
                    local_generation,
                    ..
                } => {
                    let removed_generation = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(u64::from_le_bytes)
                        .unwrap_or(0);
                    let removed = removed_generation == expected_generation;
                    if removed && local_generation != 0 {
                        // This exact local generation may already have been
                        // replaced while Raft committed the tombstone. The
                        // local name service leaves such a replacement intact.
                        let call =
                            submit_unregister_local_generation(ns_conn, &name, local_generation);
                        if call != 0 {
                            pending_local_unregistrations.push(call);
                        }
                    }
                    if reply != 0 {
                        ipc_reply(
                            reply,
                            if removed {
                                expected_generation as i64
                            } else {
                                dns::ERR_STALE_GENERATION
                            },
                        );
                    }
                }
            }
        }

        pending_local_unregistrations.retain(|call| {
            let (status, _result, _connection, _memory) = ipc_reply_poll_with_memory(*call);
            if status == 1 {
                true
            } else {
                ipc_close(*call);
                false
            }
        });

        let mut publication_index = 0;
        while publication_index < local_publications.len() {
            let publication = &mut local_publications[publication_index];
            if !publication.endpoint_closed && publication.close_watch == u64::MAX {
                publication.close_watch = ipc_connection_watch_closed(publication.connection);
            }
            if !publication.endpoint_closed && publication.close_watch != u64::MAX {
                let (status, _result) = completion_poll(publication.close_watch);
                if status == 0 {
                    completion_close(publication.close_watch);
                    ipc_close(publication.connection);
                    publication.close_watch = u64::MAX;
                    publication.connection = 0;
                    publication.endpoint_closed = true;
                    config::write::<u32>(32, 2);
                }
            }

            let still_active = catalog.lookup(&publication.name).is_some_and(|entry| {
                entry.node == node_name && entry.generation == publication.generation
            });
            if !still_active {
                if !publication.local_cleanup_submitted && publication.local_generation != 0 {
                    let call = submit_unregister_local_generation(
                        ns_conn,
                        &publication.name,
                        publication.local_generation,
                    );
                    if call != 0 {
                        pending_local_unregistrations.push(call);
                        publication.local_cleanup_submitted = true;
                    }
                }
                if publication.close_watch != u64::MAX {
                    // A replacement made this watcher obsolete. Its endpoint
                    // may still be alive, so retain the completion until that
                    // endpoint eventually closes rather than cancelling away
                    // the only strong observer reference.
                    publication_index += 1;
                } else {
                    local_publications.swap_remove(publication_index);
                }
                continue;
            }

            let now = node.millis();
            if publication.endpoint_closed && now >= publication.next_unregister_attempt {
                publication.next_unregister_attempt = now.saturating_add(AUTO_UNREGISTER_RETRY_MS);
                if node.state == NodeState::Leader {
                    let already_pending = pending_registers.iter().any(|pending| {
                        matches!(
                            pending,
                            PendingRegistration::Unregister {
                                name,
                                expected_generation,
                                automatic_term: Some(term),
                                ..
                            } if name == &publication.name
                                && *expected_generation == publication.generation
                                && *term == node.current_term
                        )
                    });
                    if !already_pending
                        && let Ok(log_index) = node.submit_command(
                            encode_unregister_generation(
                                &publication.name,
                                &node_name,
                                publication.generation,
                            ),
                            now,
                        )
                    {
                        config::write::<u32>(32, 3);
                        pending_registers.push(PendingRegistration::Unregister {
                            log_index,
                            reply: 0,
                            name: publication.name.clone(),
                            expected_generation: publication.generation,
                            local_generation: publication.local_generation,
                            automatic_term: Some(node.current_term),
                        });
                    }
                } else if let Some(leader) = node.known_leader_id.as_ref() {
                    config::write::<u32>(32, 4);
                    transport.send_message(
                        leader,
                        catten_services::runregister::TAG_REQUEST,
                        catten_services::runregister::encode_request(
                            &node_name,
                            &publication.name,
                            publication.generation,
                        ),
                    );
                }
            }
            publication_index += 1;
        }

        // A timeout cannot prove whether a remote target executed before its
        // reply was lost, so report an explicitly uncertain outcome. The
        // bounded table also prevents permanently unreachable peers from
        // growing kernel-visible pending IPC state without limit.
        let mut index = 0;
        while index < in_flight_calls.len() {
            if in_flight_calls[index].deadline > node.millis() {
                index += 1;
                continue;
            }
            let call = in_flight_calls.swap_remove(index);
            if call.reply != 0 {
                ipc_reply(call.reply, dns::ERR_UNCERTAIN);
            }
        }

        let mut index = 0;
        while index < pending_queries.len() {
            if pending_queries[index].deadline > node.millis() {
                index += 1;
                continue;
            }
            let query = pending_queries.swap_remove(index);
            let reply = match query.kind {
                PendingQueryKind::Lookup {
                    reply,
                    ..
                }
                | PendingQueryKind::Call {
                    reply,
                    ..
                } => reply,
            };
            if reply != 0 {
                // A catalog query has no target-side effect, so failure to
                // obtain the leader's read-barrier answer is safely retryable.
                ipc_reply(reply, dns::ERR_NOT_LEADER);
            }
        }

        // --- Raft clock ---
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

        config::write::<u32>(20, node.current_term as u32);
        config::write::<u32>(
            24,
            match node.state {
                NodeState::Candidate => 2,
                NodeState::Leader => 3,
                NodeState::Follower => 1,
            },
        );
        config::write::<u32>(28, catalog.registered_count() as u32);
    }
}

/// Unpack a short (<= 8 byte) service name from the packed scalar form.
fn packed_name(packed: u64) -> Vec<u8> {
    let bytes = packed.to_le_bytes();
    let len = bytes.iter().rposition(|byte| *byte != 0).map_or(0, |index| index + 1);
    bytes[..len].to_vec()
}

catten_rt::entry!(main);
