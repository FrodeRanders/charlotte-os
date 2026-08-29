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
//!
//! The service loop remains the ordering authority. Supporting modules isolate
//! catalog adaptation, transport dispatch, memory ownership, asynchronous
//! local calls, reactor maintenance phases, and the records those phases own.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    boxed::Box,
    collections::{
        BTreeMap,
        BTreeSet,
        VecDeque,
    },
    string::ToString,
    sync::Arc,
    vec,
    vec::Vec,
};

use catten_graft::{
    membership::ClusterConfiguration,
    node::RaftNode,
    types::{
        NodeState,
        Peer,
    },
};
use catten_rt::{
    Context,
    ManifestValue,
    config,
    manifest_key,
};
use catten_services::{
    broker::EventBroker,
    clusterctl,
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
        encode_deploy,
        encode_lookup_query,
        encode_register,
        encode_register_deployment,
        encode_set_cluster_key,
        encode_unregister_generation,
    },
    net,
    node_identity::{
        self,
        NodeIdentity,
    },
    ns,
    raft,
    relmsg,
    relmsg_transport::{
        RelmsgRaftTransport,
        TAG_JOIN_REPLY,
        TAG_JOIN_REQUEST,
        decode_join_reply,
        decode_join_request,
        encode_join_reply,
        encode_join_request,
    },
    wait_for_local_ready,
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
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_scalar_call_move,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
    poll as completion_poll,
    submit_detached_timer,
    thread_exit,
};
use charlotte_protocol_msg::unpack_address_and_len;

#[path = "dns/catalog.rs"]
mod catalog;
#[path = "dns/local_calls.rs"]
mod local_calls;
#[path = "dns/message_memory.rs"]
mod message_memory;
#[path = "dns/reactor.rs"]
mod reactor;
#[path = "dns/state.rs"]
mod state;
#[path = "dns/transport.rs"]
mod transport;

use catalog::{
    linearizable_entry,
    persistent_namespace,
};
use local_calls::begin_local_call;
use message_memory::{
    packed_name,
    read_call_request,
    read_deploy_request,
    read_deployment_registration,
    read_generation,
    read_key,
    read_moved_bytes,
    read_named_bytes,
    read_named_deploy_request,
    reply_move_bytes,
};
use reactor::{
    advance_raft_clock,
    drain_local_unregistrations,
    drive_local_calls,
    expire_queries,
    expire_remote_calls,
    publish_status,
};
use state::{
    CompletedCall,
    InFlightCall,
    LocalCallDestination,
    LocalPublication,
    PendingLocalCall,
    PendingQuery,
    PendingQueryKind,
    PendingRegistration,
};
use transport::{
    drive_inbound,
    query_disco_peers,
};

const LOOP_TICK_MS: u64 = 25;
const RAFT_TIMER_COOKIE: u64 = 0x444e_535f_5449_434b;
const REPLY_SPINS: u64 = u64::MAX;

const CLUSTER_KEY: u64 = manifest_key(b"cluster");
const ELECTION_KEY: u64 = manifest_key(b"elect-ms");
const DISCO_QUERY_MS: u64 = 2_000;
const JOIN_RETRY_MS: u64 = 1_000;
const REMOTE_CALL_TIMEOUT_MS: u64 = 5_000;
const MAX_IN_FLIGHT_CALLS: usize = 64;
const DEDUP_WINDOW: usize = 128;

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
    let (list_scratch_9_map_status, list_scratch_9_vaddr) = memory_map_any(cap, true);
    if cap != 0 && list_scratch_9_map_status == 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(
                entry.node.as_ptr(),
                list_scratch_9_vaddr as *mut u8,
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

fn fatal(stage: u64) -> ! {
    catten_syscall::el0_log(0x444e_5300, stage);
    unsafe { thread_exit() }
}

/// Resolve the local name-service registration for `name`: either the
/// caller-attached connection or the local registration looked up by name.
/// Returns `(connection, local_generation)`.
fn local_publication(ns_conn: u64, attached_connection: u64, name: &[u8]) -> Option<(u64, u64)> {
    if attached_connection != 0 {
        return Some((attached_connection, 0));
    }
    let lookup = ipc_scalar_call(ns_conn, ns::OP_TRY_LOOKUP, catten_services::name(name));
    if lookup == 0 {
        return None;
    }
    let (generation, connection) = unsafe { wait_reply(lookup, REPLY_SPINS) };
    if generation >= 1 && connection != 0 {
        Some((connection, generation as u64))
    } else {
        None
    }
}

/// The register/relay/submit path shared by `OP_REGISTER` and
/// `OP_REGISTER_NAMED`. Returns `Some(code)` when the caller must reply with
/// `code`, or `None` when the reply was deferred (the entry is committing).
#[allow(clippy::too_many_arguments)]
fn register_name(
    node: &mut RaftNode,
    ns_conn: u64,
    transport: &RelmsgRaftTransport,
    pending_registers: &mut alloc::vec::Vec<PendingRegistration>,
    node_name: &[u8],
    message: &catten_syscall::IpcMessage,
    name: alloc::vec::Vec<u8>,
    deployment_generation: u64,
) -> Option<i64> {
    if name.is_empty() {
        Some(dns::ERR_TOO_LARGE)
    } else if node.state != NodeState::Leader {
        // Remote host: the service lives on this node, but only the leader
        // may commit catalog entries. Resolve the local registration and
        // relay a register request to the leader, which commits the entry
        // naming this node as the owner. The reply is deferred until the
        // leader acknowledges (see rregister replies below).
        match local_publication(ns_conn, message.connection, &name) {
            None => Some(dns::ERR_TOO_LARGE),
            Some((connection, local_generation)) => match node.known_leader_id.clone() {
                Some(leader) if transport.has_peer(&leader) => {
                    let request = catten_services::rregister::encode_request(
                        node_name,
                        &name,
                        deployment_generation,
                    );
                    transport.send_message(
                        &leader,
                        catten_services::rregister::TAG_REQUEST,
                        request,
                    );
                    pending_registers.push(PendingRegistration::RemoteRegister {
                        reply: message.reply,
                        name,
                        connection,
                        local_generation,
                    });
                    None
                }
                _ => {
                    if connection != 0 {
                        ipc_close(connection);
                    }
                    Some(dns::ERR_NOT_LEADER)
                }
            },
        }
    } else {
        // Leader: commit the registration with this node as the owner.
        // Submit once; the reactor completes the reply once the entry has
        // replicated (see pending_registers below).
        let command = if deployment_generation == 0 {
            encode_register(&name, node_name)
        } else {
            encode_register_deployment(&name, node_name, deployment_generation)
        };
        match node.submit_command(command, node.millis()) {
            Ok(index) => {
                let (connection, existing_local_generation) =
                    local_publication(ns_conn, message.connection, &name).unwrap_or((0, 0));
                pending_registers.push(PendingRegistration::Prepare {
                    log_index: index,
                    reply: message.reply,
                    name,
                    connection,
                    existing_local_generation,
                });
                None
            }
            Err(code) => Some(code),
        }
    }
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
    let (list_scratch_map_status, list_scratch_vaddr) = memory_map_any(memory, true);
    if memory == 0 || list_scratch_map_status != 0 {
        if memory != 0 {
            memory_close(memory);
        }
        return 0;
    }
    unsafe {
        core::ptr::write_volatile(list_scratch_vaddr as *mut u64, generation);
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

/// Drain the administration face of the DNS-owned Raft node.
fn drain_raft_admin(endpoint: u64, node: &mut RaftNode) {
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

        match message.opcode {
            raft::OP_STATUS => {
                if message.memory != 0 {
                    memory_close(message.memory);
                }
                let state = match node.state {
                    NodeState::Follower => 1i64,
                    NodeState::Candidate => 2,
                    NodeState::Leader => 3,
                };
                let result =
                    state | ((node.current_term as i64) << 8) | ((node.commit_index as i64) << 32);
                if message.reply != 0 {
                    ipc_reply(message.reply, result);
                }
            }
            raft::OP_CLUSTER_STATUS => {
                if message.memory != 0 {
                    memory_close(message.memory);
                }
                let state = match node.state {
                    NodeState::Follower => 1,
                    NodeState::Candidate => 2,
                    NodeState::Leader => 3,
                };
                let mut status = [0u8; 256];
                let leader = node.known_leader_id.as_deref().unwrap_or("");
                if let Some(len) = raft::build_cluster_status(
                    &mut status,
                    state,
                    node.current_term,
                    node.commit_index,
                    node.cluster_configuration.all_members().len() as u32,
                    leader.as_bytes(),
                    node.me.id.as_bytes(),
                ) {
                    reply_move_bytes(message.reply, &status[..len]);
                } else if message.reply != 0 {
                    ipc_reply(message.reply, -1);
                }
            }
            raft::OP_ADD_SERVER => {
                let peer = read_moved_bytes(&message, 4096).and_then(|payload| {
                    let (id, service_name, learner) = raft::decode_peer_spec(&payload)?;
                    let id = core::str::from_utf8(id).ok()?.to_string();
                    if id.is_empty() {
                        return None;
                    }
                    Some(
                        if learner {
                            Peer::learner(id, service_name)
                        } else {
                            Peer::voter(id, service_name)
                        },
                    )
                });
                let result = match peer {
                    Some(peer) => node
                        .submit_join(peer, node.millis())
                        .map(|index| index as i64)
                        .unwrap_or_else(|code| code),
                    None => -1,
                };
                if message.reply != 0 {
                    ipc_reply(message.reply, result);
                }
            }
            raft::OP_REMOVE_SERVER => {
                let id = read_moved_bytes(&message, 4096).and_then(|payload| {
                    let (&len, rest) = payload.split_first()?;
                    let len = len as usize;
                    if len == 0 || rest.len() < len {
                        return None;
                    }
                    core::str::from_utf8(&rest[..len]).ok().map(ToString::to_string)
                });
                let result = match id {
                    Some(id) if node.state == NodeState::Leader => {
                        let members: Vec<Peer> = node
                            .cluster_configuration
                            .all_members()
                            .into_iter()
                            .filter(|peer| peer.id != id)
                            .cloned()
                            .collect();
                        if members.is_empty() {
                            raft::ERR_NOT_FOUND
                        } else {
                            node.submit_joint_configuration(members, node.millis())
                                .map(|index| index as i64)
                                .unwrap_or_else(|code| code)
                        }
                    }
                    Some(_) => raft::ERR_NOT_LEADER,
                    None => -1,
                };
                if message.reply != 0 {
                    ipc_reply(message.reply, result);
                }
            }
            _ => {
                if message.memory != 0 {
                    memory_close(message.memory);
                }
                if message.reply != 0 {
                    ipc_reply(message.reply, -1);
                }
            }
        }
    }
}

fn main(ctx: Context) -> ! {
    config::write_u32_release(dns::status::STAGE, 1);
    let mnemonic: Vec<u8> = match ctx.manifest_value(CLUSTER_KEY) {
        Some(ManifestValue::Bytes(raw)) if !raw.is_empty() => raw.to_vec(),
        _ => b"charlotte".to_vec(),
    };
    let election_timeout_ms = match ctx.manifest_value(ELECTION_KEY) {
        Some(ManifestValue::Unsigned(value)) => value,
        _ => 300,
    };
    // Keep heartbeats comfortably below the election timeout without
    // flooding the serialized relmsg path on a slow emulator.
    let heartbeat_interval_ms = (election_timeout_ms / 4).clamp(25, 250);

    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => fatal(1),
    };
    config::write_u32_release(dns::status::STAGE, 2);

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
    config::write_u32_release(dns::status::STAGE, 3);

    // Wait for the boot storm to settle before joining the cluster.
    if !wait_for_local_ready(ns_conn) {
        fatal(7);
    }
    config::write_u32_release(dns::status::STAGE, 4);

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
    config::write_u32_release(dns::status::STAGE, 5);

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

    // DNS owns the cluster's Raft node. Publish its administrative/status
    // face under the conventional per-node Raft name so discovery and
    // clusterctl observe and control this exact node rather than a second
    // service with an independent log.
    let raft_endpoint = ipc_endpoint_create(raft::INTERFACE, raft::VERSION, 8);
    if raft_endpoint == 0 || ipc_endpoint_bind_cq(raft_endpoint, 0) != 0 {
        fatal(18);
    }
    let raft_name = catten_services::raft_name(&node_name);
    let raft_register = ipc_scalar_call_connection(
        ns_conn,
        ns::OP_REGISTER,
        raft_name,
        raft_endpoint,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if raft_register == 0 {
        fatal(19);
    }
    let (raft_generation, _) = unsafe { wait_reply(raft_register, REPLY_SPINS) };
    if raft_generation < 1 {
        fatal(20);
    }
    config::write_u32_release(dns::status::STAGE, 6);

    // A fresh durable identity starts as a one-member cluster. Discovery only
    // supplies transient MAC routes; admission itself is a command in this
    // same durable Raft log.
    let transport = Arc::new(RelmsgRaftTransport::new(relmsg_conn));
    config::write_u32_release(dns::status::STAGE, 7);

    let me = Peer::voter(node_name_str.clone(), raft_name);
    config::write_u32_release(dns::status::PEER_COUNT, 1);

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
        me: me.clone(),
        timeout_millis: election_timeout_ms,
        log_store: Box::new(log_store),
        persistent_state: Box::new(persistent_state),
        state_machine: Some(catalog::state_machine(catalog.clone())),
        cluster_configuration: ClusterConfiguration::stable(vec![me]),
        transport: transport.clone(),
        current_millis: 0,
        snapshot_min_entries: 0,
        snapshot_chunk_bytes: 1200,
    });
    config::write_u32_release(dns::status::STAGE, 8);

    let cq = ctx.completion_queue_layout();
    let mut recv_pending: u64 = 0;
    let mut served: u32 = 0;
    let mut remote_calls_served: u32 = 0;
    let mut remote_queries_served: u32 = 0;
    let mut pending_registers: Vec<PendingRegistration> = Vec::new();
    let mut in_flight_calls: Vec<InFlightCall> = Vec::new();
    let mut completed_calls: VecDeque<CompletedCall> = VecDeque::new();
    let mut pending_local_calls: Vec<PendingLocalCall> = Vec::new();
    let mut next_reply_ordinal: BTreeMap<alloc::string::String, u64> = BTreeMap::new();
    let mut next_call_id: u64 = 1;
    let mut pending_queries: Vec<PendingQuery> = Vec::new();
    let mut pending_local_unregistrations: Vec<u64> = Vec::new();
    let mut local_publications: Vec<LocalPublication> = Vec::new();
    let mut next_query_id: u64 = 1;

    // Cluster-event waiters: reply tokens parked by OP_EVENT_WAIT for events
    // that have not fired yet. Settled from the *applied* catalog each
    // reactor iteration — the event fires when the replicated entry lands on
    // this node, never by polling order or boot timing. This is the
    // replicated service's event-broker face; the catalog is its catalog
    // face (see `catten_services::broker`).
    let mut event_waiters: catten_services::broker::KeyedWaitlist<u64> =
        catten_services::broker::KeyedWaitlist::new();
    let mut next_disco_query_ms = 0u64;
    let mut join_request_pending = false;
    let mut join_retry_at_ms = 0u64;
    let mut join_accepted = node.cluster_configuration.all_members().len() > 1;
    let mut membership_events_submitted: BTreeSet<Vec<u8>> = BTreeSet::new();
    let mut membership_event_term = 0u64;
    let mut timer_armed = submit_detached_timer(LOOP_TICK_MS, 0, RAFT_TIMER_COOKIE) != u64::MAX;
    let mut last_heartbeat_broadcast = 0u64;

    loop {
        // The detached timer owns Raft timekeeping, but inbound IPC replies
        // are not guaranteed to wake this CQ. Bound the reactor sleep to its
        // loop period so the relmsg receive queue cannot fill behind an
        // armed-but-delayed timer.
        let (_, timed_out) = cq_wait_timeout(1, LOOP_TICK_MS, 0);
        // The CQ timeout is an independent watchdog, not merely a fallback
        // for failure to *submit* the detached timer. A submitted timer can
        // have its completion delayed or dropped; ignoring the timeout while
        // `timer_armed` stayed true would then freeze Raft time forever.
        let mut tick_due = timed_out != 0;
        while let Some(completion) = unsafe { cq_read(cq.base, cq.entries) } {
            if completion.cookie == RAFT_TIMER_COOKIE {
                tick_due = true;
                timer_armed = false;
            }
        }

        // Discovery supplies routes, while membership remains an explicit
        // command in this Raft log. Of two fresh singleton nodes, the larger
        // durable identity applies to the smaller one; the deterministic
        // direction prevents two competing cross-joins.
        if join_request_pending && node.millis() >= join_retry_at_ms {
            join_request_pending = false;
            next_disco_query_ms = node.millis();
        }
        if node.millis() >= next_disco_query_ms {
            next_disco_query_ms = node.millis().saturating_add(DISCO_QUERY_MS);
            let mut anchor: Option<alloc::string::String> = None;
            for (mac, peer_node_id) in query_disco_peers(disco_conn) {
                let Ok(peer_id) = core::str::from_utf8(&peer_node_id) else {
                    continue;
                };
                if peer_id.is_empty() || peer_id == node_name_str {
                    continue;
                }
                transport.add_peer(peer_id, mac);
                if peer_id.as_bytes() < node.me.id.as_bytes()
                    && anchor.as_ref().is_none_or(|current| peer_id < current.as_str())
                {
                    anchor = Some(peer_id.to_string());
                }
            }
            if let Some(anchor) = anchor
                && !join_accepted
                && !join_request_pending
                && node.cluster_configuration.all_members().len() == 1
                && (node.state == NodeState::Leader
                    || (node.joining && node.joining_from.as_deref() == Some(anchor.as_str())))
                && let Some(payload) = encode_join_request(node.me.id.as_bytes(), raft_name)
            {
                if !node.joining {
                    node.begin_joining(anchor.clone(), node.millis());
                }
                transport.send_message(&anchor, TAG_JOIN_REQUEST, payload);
                join_request_pending = true;
                join_retry_at_ms = node.millis().saturating_add(JOIN_RETRY_MS);
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
                    let (rx_scratch_map_status, rx_scratch_vaddr) = memory_map_any(memory, false);
                    if rx_scratch_map_status == 0 {
                        let frame = unsafe {
                            core::slice::from_raw_parts(rx_scratch_vaddr as *const u8, len as usize)
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
                                    let duplicate_pending = pending_local_calls.iter().any(|call| {
                                        matches!(
                                            &call.destination,
                                            LocalCallDestination::Remote {
                                                caller: pending_caller,
                                                session: pending_session,
                                                call_id: pending_call_id,
                                                ..
                                            } if pending_caller == &caller
                                                && *pending_session == session
                                                && *pending_call_id == call_id
                                        )
                                    });

                                    if cached_result.is_none() && !duplicate_pending
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
                                    let remote_pending = pending_local_calls
                                        .iter()
                                        .filter(|call| {
                                            matches!(
                                                call.destination,
                                                LocalCallDestination::Remote { .. }
                                            )
                                        })
                                        .count();
                                    let has_dedup_capacity = completed_calls.len() + remote_pending
                                        < DEDUP_WINDOW;

                                    if !duplicate_pending {
                                        let mut reserved_reply_ordinal = None;
                                        let result = if let Some(result) = cached_result {
                                            Some(result)
                                        } else if !has_dedup_capacity {
                                            Some(dns::ERR_BUSY)
                                        } else {
                                            match catalog.lookup(&target) {
                                                Some(owner)
                                                    if owner.node == node_name
                                                        && owner.generation
                                                            == target_generation =>
                                                {
                                                    if pending_local_calls.len()
                                                        >= MAX_IN_FLIGHT_CALLS
                                                    {
                                                        Some(dns::ERR_BUSY)
                                                    } else {
                                                        let reply_ordinal = next_reply_ordinal
                                                            .entry(source_peer.clone())
                                                            .or_insert_with(|| {
                                                                transport
                                                                    .acknowledged_count_for(
                                                                        &source_peer,
                                                                        catten_services::rcall::TAG_REPLY,
                                                                    )
                                                            });
                                                        *reply_ordinal =
                                                            reply_ordinal.saturating_add(1);
                                                        reserved_reply_ordinal =
                                                            Some(*reply_ordinal);
                                                        let destination =
                                                            LocalCallDestination::Remote {
                                                                caller: caller.clone(),
                                                                session,
                                                                call_id,
                                                                target_generation,
                                                                peer: source_peer.clone(),
                                                                settled_after_ack: reserved_reply_ordinal
                                                                    .expect("reply ordinal reserved"),
                                                            };
                                                        match begin_local_call(
                                                            ns_conn,
                                                            &target,
                                                            opcode,
                                                            arg,
                                                            node.millis().saturating_add(
                                                                REMOTE_CALL_TIMEOUT_MS,
                                                            ),
                                                            destination,
                                                        ) {
                                                            Ok(call) => {
                                                                pending_local_calls.push(call);
                                                                None
                                                            }
                                                            Err(result) => Some(result),
                                                        }
                                                    }
                                                }
                                                Some(owner) if owner.node == node_name => {
                                                    Some(dns::ERR_STALE_GENERATION)
                                                }
                                                _ => Some(dns::ERR_NOT_FOUND),
                                            }
                                        };

                                        if let Some(result) = result {
                                            let settled_after_ack =
                                                reserved_reply_ordinal.unwrap_or_else(|| {
                                                    let reply_ordinal = next_reply_ordinal
                                                        .entry(source_peer.clone())
                                                        .or_insert_with(|| {
                                                            transport.acknowledged_count_for(
                                                                &source_peer,
                                                                catten_services::rcall::TAG_REPLY,
                                                            )
                                                        });
                                                    *reply_ordinal =
                                                        reply_ordinal.saturating_add(1);
                                                    *reply_ordinal
                                                });
                                            if cached_result.is_none() && has_dedup_capacity {
                                                completed_calls.push_back(CompletedCall {
                                                    caller,
                                                    session,
                                                    call_id,
                                                    result,
                                                    peer: source_peer.clone(),
                                                    settled_after_ack,
                                                });
                                            }
                                            remote_calls_served =
                                                remote_calls_served.wrapping_add(1);
    config::write_u32_release(
                                                dns::status::REMOTE_CALLS_SERVED,
                                                remote_calls_served,
                                            );
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
    config::write_u32_release(
                                        dns::status::REMOTE_QUERIES_SERVED,
                                        remote_queries_served,
                                    );
                                    let (status, entry) =
                                        match node.handle_client_query(encode_lookup_query(&name)) {
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
                                            && !matches!(
                                                query.kind,
                                                PendingQueryKind::Deploy { .. }
                                            )
                                    })
                                {
                                    let query = pending_queries.swap_remove(index);
                                    let entry =
                                        (status == 0 && generation != 0 && !owner.is_empty())
                                            .then_some(CatalogEntry {
                                                node: owner,
                                                generation,
                                                active: true,
                                                deployment_generation: 0,
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
                                                    if pending_local_calls.len()
                                                        >= MAX_IN_FLIGHT_CALLS
                                                    {
                                                        Some(dns::ERR_BUSY)
                                                    } else {
                                                        match begin_local_call(
                                                            ns_conn,
                                                            &name,
                                                            opcode,
                                                            arg,
                                                            node.millis().saturating_add(
                                                                REMOTE_CALL_TIMEOUT_MS,
                                                            ),
                                                            LocalCallDestination::Client { reply },
                                                        ) {
                                                            Ok(call) => {
                                                                pending_local_calls.push(call);
                                                                None
                                                            }
                                                            Err(result) => Some(result),
                                                        }
                                                    }
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
                                        PendingQueryKind::Deploy {
                                            reply,
                                        } => {
                                            if reply != 0 {
                                                ipc_reply(reply, dns::ERR_NOT_LEADER);
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
                            Some(catten_services::rregister::TAG_REQUEST) => {
                                // A follower hosts a service locally but only
                                // this leader may commit its catalog entry.
                                if let Some((owner, name, deployment_generation)) =
                                    catten_services::rregister::decode_request(frame)
                                    && node.state == NodeState::Leader
                                    && transport
                                        .peer_id_for_mac(&source_mac)
                                        .is_some_and(|peer| peer.as_bytes() == owner)
                                    && !pending_registers.iter().any(|pending| matches!(
                                        pending,
                                        PendingRegistration::RemotePrepare {
                                            name: pending_name,
                                            ..
                                        }
                                        | PendingRegistration::RemoteActivate {
                                            name: pending_name,
                                            ..
                                        } if pending_name == &name
                                    ))
                                    && let Ok(log_index) = node.submit_command(if deployment_generation == 0 {
                                        encode_register(&name, &owner)
                                    } else {
                                        encode_register_deployment(
                                            &name,
                                            &owner,
                                            deployment_generation,
                                        )
                                    }, node.millis())
                                {
                                    pending_registers.push(PendingRegistration::RemotePrepare {
                                        log_index,
                                        name,
                                        owner,
                                    });
                                }
                            }
                            Some(catten_services::rregister::TAG_REPLY) => {
                                // The leader acknowledged a relayed register:
                                // publish the locally hosted service.
                                if let Some((owner, name, generation)) =
                                    catten_services::rregister::decode_reply(frame)
                                    && owner == node_name
                                    && let Some(index) =
                                        pending_registers.iter().position(|pending| matches!(
                                            pending,
                                            PendingRegistration::RemoteRegister {
                                                name: pending_name,
                                                ..
                                            } if pending_name == &name
                                        ))
                                {
                                    let PendingRegistration::RemoteRegister {
                                        reply,
                                        connection,
                                        local_generation,
                                        ..
                                    } = pending_registers.swap_remove(index)
                                    else {
                                        unreachable!()
                                    };
                                    if generation >= 1 && connection != 0 {
                                        let close_watch = ipc_connection_watch_closed(connection);
    config::write_u32_release(
                                            dns::status::PUBLICATION_LIFECYCLE,
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
                                            if generation >= 1 {
                                                generation as i64
                                            } else {
                                                dns::ERR_NOT_FOUND
                                            },
                                        );
                                    }
                                }
                            }
                            Some(catten_services::rdeploy::TAG_REQUEST) => {
                                if let Some(request) = catten_services::rdeploy::decode_request(frame)
                                    && let Some(source_peer) =
                                        transport.peer_id_for_mac(&source_mac)
                                    && source_peer.as_bytes() == request.caller
                                {
                                    let result = if node.state != NodeState::Leader {
                                        Some(dns::ERR_NOT_LEADER)
                                    } else if !charlotte_launch::deployment::valid_artifact_name(
                                        &request.artifact,
                                    ) || request.descriptor.len()
                                        > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN
                                    {
                                        Some(dns::ERR_TOO_LARGE)
                                    } else {
                                        let assigned_node = if request.node_key == 0 {
                                            node_identity::key_from_name(&node_name).unwrap_or(0)
                                        } else {
                                            request.node_key
                                        };
                                        match node.submit_command(
                                            encode_deploy(
                                                &request.artifact,
                                                request.object_id,
                                                assigned_node,
                                                &request.digest,
                                                &request.descriptor,
                                            ),
                                            node.millis(),
                                        ) {
                                            Ok(log_index) => {
                                                pending_registers.push(
                                                    PendingRegistration::RemoteDeploy {
                                                        log_index,
                                                        peer: source_peer.clone(),
                                                        session: request.session,
                                                        request_id: request.request_id,
                                                    },
                                                );
                                                None
                                            }
                                            Err(code) => Some(code),
                                        }
                                    };
                                    if let Some(result) = result {
                                        transport.send_message(
                                            &source_peer,
                                            catten_services::rdeploy::TAG_REPLY,
                                            catten_services::rdeploy::encode_reply(
                                                request.session,
                                                request.request_id,
                                                result,
                                            ),
                                        );
                                    }
                                }
                            }
                            Some(catten_services::rdeploy::TAG_REPLY) => {
                                if let Some((session, request_id, result)) =
                                    catten_services::rdeploy::decode_reply(frame)
                                    && session == dns_session
                                    && let Some(source_peer) =
                                        transport.peer_id_for_mac(&source_mac)
                                    && let Some(index) =
                                        pending_queries.iter().position(|query| {
                                            query.query_id == request_id
                                                && query.expected_leader == source_peer
                                                && matches!(
                                                    query.kind,
                                                    PendingQueryKind::Deploy { .. }
                                                )
                                        })
                                {
                                    let query = pending_queries.swap_remove(index);
                                    let PendingQueryKind::Deploy {
                                        reply,
                                    } = query.kind
                                    else {
                                        unreachable!()
                                    };
                                    if reply != 0 {
                                        ipc_reply(reply, result);
                                    }
                                }
                            }
                            Some(TAG_JOIN_REQUEST) => {
                                let accepted = decode_join_request(&frame[1..])
                                    .and_then(|(joiner_id, service_name)| {
                                        let joiner = core::str::from_utf8(joiner_id).ok()?;
                                        let route_matches = transport
                                            .peer_id_for_mac(&source_mac)
                                            .is_some_and(|peer| peer == joiner);
                                        if node.state != NodeState::Leader
                                            || joiner.is_empty()
                                            || !route_matches
                                        {
                                            return Some(0);
                                        }
                                        Some(
                                            node.submit_join(
                                                Peer::voter(joiner.to_string(), service_name),
                                                node.millis(),
                                            )
                                            .unwrap_or(0),
                                        )
                                    })
                                    .unwrap_or(0);
                                transport.send_response(
                                    source_mac,
                                    TAG_JOIN_REPLY,
                                    encode_join_reply(accepted),
                                );
                            }
                            Some(TAG_JOIN_REPLY) => {
                                if let Some(index) = decode_join_reply(&frame[1..]) {
                                    join_request_pending = false;
                                    if index > 0 {
                                        join_accepted = true;
                                    }
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
        config::write_u32_release(
            dns::status::REMOTE_CALL_ACKS,
            transport.acknowledged_count(catten_services::rcall::TAG_REPLY).min(u32::MAX as u64)
                as u32,
        );
        config::write_u32_release(
            dns::status::REMOTE_QUERY_REPLY_ACKS,
            transport.acknowledged_count(catten_services::rquery::TAG_REPLY).min(u32::MAX as u64)
                as u32,
        );

        let completed = node.poll_transport(node.millis());
        if completed > 0 {
            config::write_u32_release(dns::status::TRANSPORT_COMPLETIONS, completed as u32);
        }

        // --- Cluster events ---
        // Publish completed admissions through the DNS state machine. The
        // configuration transition and its observable membership event are
        // therefore ordered in one Raft log.
        if node.state == NodeState::Leader
            && !node.cluster_configuration.is_joint_consensus()
            && node.cluster_configuration.current_members().len() > 1
        {
            if membership_event_term != node.current_term {
                membership_event_term = node.current_term;
                membership_events_submitted.clear();
                pending_registers.retain(|pending| {
                    !matches!(
                        pending,
                        PendingRegistration::Prepare {
                            reply: 0,
                            connection: 0,
                            name,
                            ..
                        } if name.starts_with(b"event:membership:")
                    )
                });
            }
            let member_ids: Vec<alloc::string::String> = node
                .cluster_configuration
                .current_members()
                .into_iter()
                .filter(|peer| peer.id != node.me.id)
                .map(|peer| peer.id.clone())
                .collect();
            for member_id in member_ids {
                let name = alloc::format!("event:membership:{member_id}").into_bytes();
                if catalog.lookup(&name).is_none()
                    && !membership_events_submitted.contains(&name)
                    && let Ok(log_index) =
                        node.submit_command(encode_register(&name, &node_name), node.millis())
                {
                    membership_events_submitted.insert(name.clone());
                    pending_registers.push(PendingRegistration::Prepare {
                        log_index,
                        reply: 0,
                        name,
                        connection: 0,
                        existing_local_generation: 0,
                    });
                }
            }
        }

        // Settle event-broker waiters from the *applied* catalog: any entry
        // that landed in this iteration (via replication or a local commit)
        // fires its waiters. Fulfillment is defined by consensus, never by
        // polling order.
        let settled = event_waiters.settle(&*catalog);
        if !settled.is_empty() {
            for (name, reply) in settled {
                if reply != 0 {
                    match catalog.lookup(&name) {
                        Some(entry) => ipc_reply(reply, entry.generation as i64),
                        None => ipc_reply(reply, dns::ERR_NOT_FOUND),
                    };
                }
            }
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
            config::write_u32_release(dns::status::IPC_REQUESTS_SERVED, served);
            match message.opcode {
                dns::OP_REGISTER => {
                    let name = packed_name(message.arg0);
                    if let Some(result) = register_name(
                        &mut node,
                        ns_conn,
                        &transport,
                        &mut pending_registers,
                        &node_name,
                        &message,
                        name,
                        0,
                    ) && message.reply != 0
                    {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_REGISTER_NAMED => {
                    if let Some(name) = read_named_bytes(&message)
                        && let Some(result) = register_name(
                            &mut node,
                            ns_conn,
                            &transport,
                            &mut pending_registers,
                            &node_name,
                            &message,
                            name,
                            0,
                        )
                        && message.reply != 0
                    {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_REGISTER_DEPLOYMENT_NAMED => {
                    if let Some((name, deployment_generation)) =
                        read_deployment_registration(&message)
                        && let Some(result) = register_name(
                            &mut node,
                            ns_conn,
                            &transport,
                            &mut pending_registers,
                            &node_name,
                            &message,
                            name,
                            deployment_generation,
                        )
                        && message.reply != 0
                    {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_EVENT_FIRE => {
                    // Commit a cluster event to the replicated catalog.
                    // Catalog-only: the event has no local service to
                    // publish, so the entry carries no connection and the
                    // local name service is untouched. On a follower the
                    // event relays to the leader through the same machinery
                    // as registrations; the reply is deferred until the
                    // entry replicates (pending_registers).
                    if let Some(name) = read_named_bytes(&message) {
                        let result = if node.state != NodeState::Leader {
                            match node.known_leader_id.clone() {
                                Some(leader) if transport.has_peer(&leader) => {
                                    let request = catten_services::rregister::encode_request(
                                        &node_name, &name, 0,
                                    );
                                    transport.send_message(
                                        &leader,
                                        catten_services::rregister::TAG_REQUEST,
                                        request,
                                    );
                                    pending_registers.push(PendingRegistration::RemoteRegister {
                                        reply: message.reply,
                                        name,
                                        connection: 0,
                                        local_generation: 0,
                                    });
                                    continue;
                                }
                                _ => dns::ERR_NOT_LEADER,
                            }
                        } else {
                            match node
                                .submit_command(encode_register(&name, &node_name), node.millis())
                            {
                                Ok(index) => {
                                    pending_registers.push(PendingRegistration::Prepare {
                                        log_index: index,
                                        reply: message.reply,
                                        name,
                                        connection: 0,
                                        existing_local_generation: 0,
                                    });
                                    continue;
                                }
                                Err(code) => code,
                            }
                        };
                        if message.reply != 0 {
                            ipc_reply(message.reply, result);
                        }
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_TOO_LARGE);
                    }
                }

                dns::OP_EVENT_WAIT => {
                    // Cluster-event wait: the event name travels in the
                    // moved memory object (it exceeds the packed-8-byte
                    // scalar limit). If the event has fired — the name is in
                    // the *applied* catalog — reply with its generation now;
                    // otherwise the event broker parks the reply token and
                    // the reactor settles it when the replicated entry lands.
                    if let Some(name) = read_named_bytes(&message) {
                        if let Some(reply) = event_waiters.park(&name, message.reply, &*catalog) {
                            if let Some(entry) = catalog.lookup(&name) {
                                if reply != 0 {
                                    ipc_reply(reply, entry.generation as i64);
                                }
                            } else if reply != 0 {
                                ipc_reply(reply, dns::ERR_NOT_FOUND);
                            }
                        }
                        continue;
                    }
                    if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_TOO_LARGE);
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

                dns::OP_DEPLOY => {
                    let artifact = packed_name(message.arg0);
                    let request = read_deploy_request(&message);
                    let result = if artifact.is_empty() {
                        dns::ERR_TOO_LARGE
                    } else if node.state != NodeState::Leader {
                        dns::ERR_NOT_LEADER
                    } else {
                        match request {
                            Some((object_id, node_key, artifact_digest, descriptor)) => {
                                // A cluster decision: commit the assignment
                                // to the replicated manifest. Its
                                // authenticity is the Raft consensus; the
                                // reply is deferred until the command is
                                // committed (pending_registers below).
                                match node.submit_command(
                                    encode_deploy(
                                        &artifact,
                                        object_id,
                                        if node_key == 0 {
                                            node_identity::key_from_name(&node_name).unwrap_or(0)
                                        } else {
                                            node_key
                                        },
                                        &artifact_digest,
                                        &descriptor,
                                    ),
                                    node.millis(),
                                ) {
                                    Ok(log_index) => {
                                        pending_registers.push(PendingRegistration::Deploy {
                                            log_index,
                                            reply: message.reply,
                                        });
                                        continue;
                                    }
                                    Err(code) => code,
                                }
                            }
                            None => dns::ERR_TOO_LARGE,
                        }
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_DEPLOY_NAMED => {
                    let request = read_named_deploy_request(&message);
                    let result = match request {
                        Some(request) if node.state == NodeState::Leader => {
                            match node.submit_command(
                                encode_deploy(
                                    &request.name,
                                    request.object_id,
                                    if request.node_key == 0 {
                                        node_identity::key_from_name(&node_name).unwrap_or(0)
                                    } else {
                                        request.node_key
                                    },
                                    &request.digest,
                                    &request.descriptor,
                                ),
                                node.millis(),
                            ) {
                                Ok(log_index) => {
                                    pending_registers.push(PendingRegistration::Deploy {
                                        log_index,
                                        reply: message.reply,
                                    });
                                    continue;
                                }
                                Err(code) => code,
                            }
                        }
                        Some(request) => {
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
                                let request_id = next_query_id;
                                next_query_id = next_query_id.wrapping_add(1).max(1);
                                let relay = catten_services::rdeploy::Request {
                                    session: dns_session,
                                    request_id,
                                    caller: node_name.clone(),
                                    artifact: request.name,
                                    object_id: request.object_id,
                                    node_key: request.node_key,
                                    digest: request.digest,
                                    descriptor: request.descriptor,
                                };
                                let Some(frame) = catten_services::rdeploy::encode_request(&relay)
                                else {
                                    if message.reply != 0 {
                                        ipc_reply(message.reply, dns::ERR_TOO_LARGE);
                                    }
                                    continue;
                                };
                                pending_queries.push(PendingQuery {
                                    query_id: request_id,
                                    expected_leader: leader.clone(),
                                    deadline: node.millis().saturating_add(REMOTE_CALL_TIMEOUT_MS),
                                    kind: PendingQueryKind::Deploy {
                                        reply: message.reply,
                                    },
                                });
                                transport.send_message(
                                    &leader,
                                    catten_services::rdeploy::TAG_REQUEST,
                                    frame,
                                );
                                continue;
                            }
                        }
                        None => dns::ERR_TOO_LARGE,
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_DEPLOY_QUERY => {
                    let artifact = packed_name(message.arg0);
                    if artifact.is_empty() {
                        if message.reply != 0 {
                            ipc_reply(message.reply, dns::ERR_TOO_LARGE);
                        }
                        continue;
                    }
                    // Answered from locally applied cluster state: a
                    // deployment is only visible here once the log entry has
                    // replicated to this replica. Agents poll, so no read
                    // barrier is required.
                    if let Some(entry) = catalog.deployment(&artifact) {
                        let mut bytes = Vec::with_capacity(60 + entry.descriptor.len());
                        bytes.extend_from_slice(&entry.generation.to_le_bytes());
                        bytes.extend_from_slice(&entry.object_id.to_le_bytes());
                        bytes.extend_from_slice(&entry.node_key.to_le_bytes());
                        bytes.extend_from_slice(&entry.artifact_digest);
                        bytes.extend_from_slice(&(entry.descriptor.len() as u32).to_le_bytes());
                        bytes.extend_from_slice(&entry.descriptor);
                        reply_move_bytes(message.reply, &bytes);
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_NOT_FOUND);
                    }
                    continue;
                }

                dns::OP_DEPLOY_QUERY_NAMED => {
                    let artifact = read_named_bytes(&message);
                    if let Some(artifact) = artifact
                        && let Some(entry) = catalog.deployment(&artifact)
                    {
                        let mut bytes = Vec::with_capacity(60 + entry.descriptor.len());
                        bytes.extend_from_slice(&entry.generation.to_le_bytes());
                        bytes.extend_from_slice(&entry.object_id.to_le_bytes());
                        bytes.extend_from_slice(&entry.node_key.to_le_bytes());
                        bytes.extend_from_slice(&entry.artifact_digest);
                        bytes.extend_from_slice(&(entry.descriptor.len() as u32).to_le_bytes());
                        bytes.extend_from_slice(&entry.descriptor);
                        reply_move_bytes(message.reply, &bytes);
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_NOT_FOUND);
                    }
                    continue;
                }

                dns::OP_DEPLOY_LIST => {
                    let deployments = catalog.deployments();
                    let mut bytes = Vec::with_capacity(
                        2 + deployments.iter().map(|(name, _)| 1 + name.len()).sum::<usize>(),
                    );
                    bytes.extend_from_slice(&(deployments.len() as u16).to_le_bytes());
                    for (name, _) in deployments {
                        let Ok(name_len) = u8::try_from(name.len()) else {
                            continue;
                        };
                        bytes.push(name_len);
                        bytes.extend_from_slice(&name);
                    }
                    reply_move_bytes(message.reply, &bytes);
                    continue;
                }

                dns::OP_DEPLOY_ROLLOUT_NAMED => {
                    let artifact = read_named_bytes(&message);
                    if let Some(artifact) = artifact
                        && let Some(deployment) = catalog.deployment(&artifact)
                    {
                        let owner = catalog.lookup(&artifact);
                        let service_generation = owner.as_ref().map_or(0, |entry| entry.generation);
                        let state = match owner.as_ref() {
                            None => clusterctl::ROLLOUT_COMMITTED,
                            Some(entry)
                                if entry.deployment_generation == deployment.generation
                                    && node_identity::key_from_name(&entry.node)
                                        == Some(deployment.node_key) =>
                            {
                                clusterctl::ROLLOUT_READY
                            }
                            Some(_) => clusterctl::ROLLOUT_REPLACING,
                        };
                        let status = clusterctl::RolloutStatus {
                            state,
                            deployment_generation: deployment.generation,
                            service_generation,
                            node_key: deployment.node_key,
                        };
                        reply_move_bytes(message.reply, &status.encode());
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_NOT_FOUND);
                    }
                    continue;
                }

                dns::OP_SET_KEY => {
                    let result = if node.state != NodeState::Leader {
                        dns::ERR_NOT_LEADER
                    } else {
                        match read_key(&message) {
                            Some(key) if key == charlotte_launch::CLUSTER_PUBLIC_KEY => {
                                // The key ceremony: commit the cluster's
                                // public key to the replicated state. The
                                // reply is deferred until it has committed.
                                match node
                                    .submit_command(encode_set_cluster_key(&key), node.millis())
                                {
                                    Ok(log_index) => {
                                        pending_registers.push(PendingRegistration::SetKey {
                                            log_index,
                                            reply: message.reply,
                                        });
                                        continue;
                                    }
                                    Err(code) => code,
                                }
                            }
                            Some(_) => dns::ERR_UNTRUSTED_KEY,
                            None => dns::ERR_TOO_LARGE,
                        }
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_KEY => {
                    // Answered from locally applied state: the ceremony's
                    // record replicates to every node.
                    if let Some(key) = catalog.cluster_key() {
                        reply_move_bytes(message.reply, &key);
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_NOT_FOUND);
                    }
                    continue;
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
                    let (catalog_scratch_map_status, catalog_scratch_vaddr) =
                        memory_map_any(cap, true);
                    if catalog_scratch_map_status != 0 {
                        memory_close(cap);
                        if message.reply != 0 {
                            ipc_reply(message.reply, dns::ERR_BAD_OPCODE);
                        }
                        continue;
                    }
                    let entries = catalog.entries();
                    let mut length = dns::CATALOG_HEADER_BYTES;
                    unsafe {
                        core::ptr::write_volatile(
                            catalog_scratch_vaddr as *mut u32,
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
                                (catalog_scratch_vaddr + length) as *mut u8,
                                name_len as u8,
                            );
                            core::ptr::copy_nonoverlapping(
                                name.as_ptr(),
                                (catalog_scratch_vaddr + length + 1) as *mut u8,
                                name_len,
                            );
                            core::ptr::write_volatile(
                                (catalog_scratch_vaddr + length + 1 + name_len) as *mut u8,
                                node_len as u8,
                            );
                            core::ptr::copy_nonoverlapping(
                                entry.node.as_ptr(),
                                (catalog_scratch_vaddr + length + 2 + name_len) as *mut u8,
                                node_len,
                            );
                            core::ptr::copy_nonoverlapping(
                                entry.generation.to_le_bytes().as_ptr(),
                                (catalog_scratch_vaddr + length + 2 + name_len + node_len)
                                    as *mut u8,
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
                                if pending_local_calls.len() >= MAX_IN_FLIGHT_CALLS {
                                    dns::ERR_BUSY
                                } else {
                                    match begin_local_call(
                                        ns_conn,
                                        &name,
                                        opcode,
                                        arg,
                                        node.millis().saturating_add(REMOTE_CALL_TIMEOUT_MS),
                                        LocalCallDestination::Client {
                                            reply: message.reply,
                                        },
                                    ) {
                                        Ok(call) => {
                                            pending_local_calls.push(call);
                                            continue;
                                        }
                                        Err(result) => result,
                                    }
                                }
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

        // Discovery and clusterctl use this administrative face, but all of
        // its operations target the DNS-owned node above.
        drain_raft_admin(raft_endpoint, &mut node);

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
                // Completed by the rregister reply frame, not by a log entry.
                PendingRegistration::RemoteRegister {
                    ..
                } => {
                    index += 1;
                    continue;
                }
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
                }
                | PendingRegistration::Deploy {
                    log_index,
                    ..
                }
                | PendingRegistration::RemoteDeploy {
                    log_index,
                    ..
                }
                | PendingRegistration::SetKey {
                    log_index,
                    ..
                }
                | PendingRegistration::RemotePrepare {
                    log_index,
                    ..
                }
                | PendingRegistration::RemoteActivate {
                    log_index,
                    ..
                } => *log_index,
            };
            if !node.is_committed(log_index) {
                index += 1;
                continue;
            }
            match pending_registers.swap_remove(index) {
                // Skipped above (completed by the rregister reply frame).
                PendingRegistration::RemoteRegister {
                    ..
                } => unreachable!(),
                PendingRegistration::Deploy {
                    reply,
                    ..
                } => {
                    // The deployment is committed and replicated: report the
                    // manifest generation to the deployer.
                    let generation = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(u64::from_le_bytes)
                        .unwrap_or(0);
                    if reply != 0 {
                        ipc_reply(reply, generation as i64);
                    }
                }
                PendingRegistration::RemoteDeploy {
                    peer,
                    session,
                    request_id,
                    ..
                } => {
                    let generation = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(u64::from_le_bytes)
                        .unwrap_or(0);
                    transport.send_message(
                        &peer,
                        catten_services::rdeploy::TAG_REPLY,
                        catten_services::rdeploy::encode_reply(
                            session,
                            request_id,
                            if generation == 0 {
                                dns::ERR_NOT_FOUND
                            } else {
                                generation as i64
                            },
                        ),
                    );
                }
                PendingRegistration::SetKey {
                    reply,
                    ..
                } => {
                    let generation = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(u64::from_le_bytes)
                        .unwrap_or(0);
                    if reply != 0 {
                        ipc_reply(reply, generation as i64);
                    }
                }
                PendingRegistration::RemotePrepare {
                    name,
                    owner,
                    ..
                } => {
                    // Register half committed: activate it, then relay the
                    // generation back to the hosting node. A failed activate
                    // (leadership lost mid-flow) is reported as generation 0
                    // so the host's caller does not hang.
                    let generation = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(u64::from_le_bytes)
                        .unwrap_or(0);
                    if generation >= 1
                        && let Ok(activate_index) =
                            node.submit_command(encode_activate(&name, generation), node.millis())
                    {
                        pending_registers.push(PendingRegistration::RemoteActivate {
                            log_index: activate_index,
                            name,
                            owner,
                            generation,
                        });
                    } else {
                        let reply = catten_services::rregister::encode_reply(&owner, &name, 0);
                        let owner = alloc::string::String::from_utf8_lossy(&owner);
                        transport.send_message(
                            &owner,
                            catten_services::rregister::TAG_REPLY,
                            reply,
                        );
                    }
                }
                PendingRegistration::RemoteActivate {
                    name,
                    owner,
                    generation,
                    ..
                } => {
                    let activated = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(u64::from_le_bytes)
                        == Some(generation);
                    let reply = catten_services::rregister::encode_reply(
                        &owner,
                        &name,
                        if activated {
                            generation
                        } else {
                            0
                        },
                    );
                    let owner = alloc::string::String::from_utf8_lossy(&owner);
                    transport.send_message(&owner, catten_services::rregister::TAG_REPLY, reply);
                }
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
                        config::write_u32_release(
                            dns::status::PUBLICATION_LIFECYCLE,
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

        drain_local_unregistrations(&mut pending_local_unregistrations);

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
                    config::write_u32_release(dns::status::PUBLICATION_LIFECYCLE, 2);
                }
            }

            let still_active = catalog.lookup(&publication.name).is_some_and(|entry| {
                entry.node == node_name && entry.generation == publication.generation
            });
            // A publication becomes stale only when the catalog has moved past
            // its generation (a replacement or migration) or when this dns
            // itself tore the endpoint down. An absent entry is NOT stale by
            // itself: for a follower the activate may simply still be
            // replicating when the leader's register reply arrives, and
            // cleaning up there would unregister a live local service.
            let superseded = catalog.lookup(&publication.name).is_some_and(|entry| {
                entry.generation != publication.generation || entry.node != node_name
            });
            if !still_active && (superseded || publication.endpoint_closed) {
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
                        config::write_u32_release(dns::status::PUBLICATION_LIFECYCLE, 3);
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
                    config::write_u32_release(dns::status::PUBLICATION_LIFECYCLE, 4);
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

        drive_local_calls(
            &mut pending_local_calls,
            &mut completed_calls,
            &mut remote_calls_served,
            &transport,
            node.millis(),
        );
        expire_remote_calls(&mut in_flight_calls, node.millis());
        expire_queries(&mut pending_queries, node.millis());
        advance_raft_clock(
            &mut node,
            tick_due,
            heartbeat_interval_ms,
            &mut last_heartbeat_broadcast,
            &mut timer_armed,
        );
        publish_status(&node, &catalog);
    }
}

catten_rt::entry!(main);
