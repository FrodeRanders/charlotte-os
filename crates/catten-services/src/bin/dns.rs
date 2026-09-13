//! Distributed name service (`dns`) — a Raft-replicated `name -> node` catalog.
//!
//! One replica runs per node. Each replica:
//! - derives its persistent node identity from the NIC MAC + cluster mnemonic ([`NodeIdentity`])
//!   and waits for the kernel's boot-done marker,
//! - discovers its peers through the cluster discovery service (`disco`),
//! - runs a [`RaftNode`] whose operational RPCs use direct Ethernet while admission and
//!   application/control traffic use the reliable message layer ([`RelmsgRaftTransport`]), and
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
        btree_map::Entry,
    },
    string::ToString,
    sync::Arc,
    vec,
    vec::Vec,
};

use catten_graft::{
    membership::ClusterConfiguration,
    node::RaftNode,
    transport::RaftTransport,
    types::{
        NodeState,
        Peer,
    },
};
use catten_rt::{
    Context,
    ManifestValue,
    ShutdownRequest,
    config,
    manifest_key,
    owned::{
        CallResult,
        ConnectionRef,
        PendingCall,
    },
};
use catten_services::{
    broker::EventBroker,
    cluster_ingress::{
        Backend,
        BackendSnapshot,
        load_balancing_epoch,
        service_load_balancing_epoch,
    },
    cluster_observe,
    clusterctl,
    disco,
    disk_raft::{
        DiskLogStore,
        DiskPersistentStateStore,
    },
    dns,
    entropy,
    frouter,
    name_catalog::{
        CatalogEntry,
        NameCatalog,
        NodeCapacityEntry,
        decode_query_result,
        encode_activate,
        encode_deploy,
        encode_deployment_result,
        encode_ingress_policy,
        encode_lookup_query,
        encode_node_capacity,
        encode_reassign,
        encode_register,
        encode_register_deployment,
        encode_set_cluster_key,
        encode_shutdown,
        encode_unregister_generation,
    },
    net,
    node_identity::{
        self,
        NodeIdentity,
    },
    ns,
    operations_admission,
    raft,
    relmsg,
    relmsg_transport::{
        RelmsgRaftTransport,
        TAG_APPEND_REQUEST,
        TAG_APPEND_RESPONSE,
        TAG_JOIN_REPLY,
        TAG_JOIN_REQUEST,
        TAG_SNAPSHOT_RESPONSE,
        TAG_VOTE_REQUEST,
        decode_join_reply,
        decode_join_request,
        encode_join_reply,
        encode_join_request,
    },
    time,
    wait_for_local_ready_or_shutdown,
    wait_for_registered_name_owned,
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
    memory_size,
    memory_unmap,
    poll as completion_poll,
    submit_detached_timer,
    thread_exit,
};
use charlotte_protocol_msg::{
    IPC_MESSAGE_HEADER_SIZE,
    parse_ipc_message_header,
};

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

const CLUSTER_KEY: u64 = manifest_key(b"cluster");
const ELECTION_KEY: u64 = manifest_key(b"elect-ms");
const INGRESS_SERVICES_KEY: u64 = manifest_key(b"vips");
const DISCO_QUERY_MS: u64 = 2_000;
// Keep retry slower than the relmsg acknowledgement/retry lease. JOIN is
// idempotent, but flooding duplicates ahead of AppendEntries can otherwise
// starve heartbeats precisely while a two-voter configuration is fragile.
const JOIN_RETRY_MS: u64 = 5_000;

fn join_request_allowed(
    committed_members: usize,
    singleton_leader: bool,
    joining_from_anchor: bool,
) -> bool {
    (committed_members == 1 && singleton_leader) || joining_from_anchor
}
const REMOTE_CALL_TIMEOUT_MS: u64 = 5_000;
/// Upper bound on deferred admissions awaiting commit. Entries beyond this
/// bound are failed as uncertain so the queue cannot grow without limit.
const MAX_PENDING_REGISTRATIONS: usize = 256;
const REMOTE_OPERATIONS_TIMEOUT_MS: u64 = 60_000;
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
            unsafe { wait_reply(lookup) }
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
    let (generation, connection) = unsafe { wait_reply(lookup) };
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
                        term: node.current_term,
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
                    term: node.current_term,
                    log_index: index,
                    reply: message.reply,
                    name,
                    connection,
                    existing_local_generation,
                });
                None
            }
            Err(code) => {
                if message.connection != 0 {
                    ipc_close(message.connection);
                }
                Some(code)
            }
        }
    }
}

fn local_generation(ns_conn: u64, name: &[u8]) -> u64 {
    let lookup = ipc_scalar_call(ns_conn, ns::OP_TRY_LOOKUP, catten_services::name(name));
    if lookup == 0 {
        return 0;
    }
    let (generation, connection) = unsafe { wait_reply(lookup) };
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

/// Materialize one all-or-nothing view of committed ingress eligibility.
/// Discovery supplies only the Ethernet route for each already-admitted
/// identity. If any route is missing, callers retain their previous snapshot
/// instead of selecting from inconsistent partial sets on different nodes.
fn ingress_membership_snapshot(
    node: &RaftNode,
    transport: &RelmsgRaftTransport,
    catalog: &NameCatalog,
    local_mac: [u8; 6],
    service_name: Option<&[u8]>,
) -> Option<BackendSnapshot> {
    let self_node = node_identity::key_from_name(node.me.id.as_bytes())?;
    let mut members = Vec::new();
    for peer in node.cluster_configuration.active_voting_members() {
        let node_id = node_identity::key_from_name(peer.id.as_bytes())?;
        let mac = if peer.id == node.me.id {
            local_mac
        } else {
            transport.mac_for_peer(&peer.id)?
        };
        members.push(Backend {
            node_id,
            mac,
        });
    }
    let mut draining = catalog
        .ingress_draining_nodes()
        .into_iter()
        .filter(|(node_id, _)| members.iter().any(|member| member.node_id == *node_id))
        .collect::<Vec<_>>();
    draining.sort_unstable_by_key(|(node_id, _)| *node_id);
    let ingress_nodes = members
        .iter()
        .filter(|member| {
            draining.binary_search_by_key(&member.node_id, |(node_id, _)| *node_id).is_err()
        })
        .map(|member| member.node_id)
        .collect::<Vec<_>>();
    let (deployment_generation, service_generation, placed_nodes) = service_name.map_or_else(
        || (0, 0, None),
        |name| {
            catalog.ingress_placement(name).map_or((0, 0, Some(Vec::new())), |placement| {
                (
                    placement.deployment_generation,
                    placement.service_generation,
                    Some(placement.ready_nodes),
                )
            })
        },
    );
    let eligible_nodes = placed_nodes.map_or_else(
        || ingress_nodes.clone(),
        |placed| {
            ingress_nodes
                .iter()
                .copied()
                .filter(|node_id| placed.contains(node_id))
                .collect::<Vec<_>>()
        },
    );
    let leader = if node.state == NodeState::Leader {
        Some(node.me.id.as_str())
    } else {
        node.known_leader_id.as_deref()
    };
    let advertiser_node = leader
        .filter(|leader| {
            node.cluster_configuration
                .active_voting_members()
                .iter()
                .any(|peer| peer.id.as_str() == *leader)
        })
        .and_then(|leader| node_identity::key_from_name(leader.as_bytes()))
        .filter(|leader| ingress_nodes.contains(leader))
        .or_else(|| ingress_nodes.iter().copied().min())
        .filter(|_| !eligible_nodes.is_empty());
    let assignment_sequence = catalog
        .ingress_policy()
        .and_then(|entry| {
            charlotte_launch::ingress_policy::decode(&entry.envelope).map(|policy| policy.sequence)
        })
        .unwrap_or(0);
    let epoch = service_name.map_or_else(
        || load_balancing_epoch(node.membership_epoch(), &draining),
        |name| {
            service_load_balancing_epoch(
                node.membership_epoch(),
                name,
                assignment_sequence,
                deployment_generation,
                service_generation,
                &eligible_nodes,
                &draining,
            )
        },
    );
    BackendSnapshot::new_with_members(epoch, self_node, advertiser_node, members, eligible_nodes)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EffectiveIngressBinding {
    service: charlotte_launch::ingress::ServiceId,
    backend_name: Option<Vec<u8>>,
}

fn effective_ingress_bindings(
    catalog: &NameCatalog,
    bootstrap: &[charlotte_launch::ingress::ServiceBinding<'_>],
) -> Vec<EffectiveIngressBinding> {
    if let Some(entry) = catalog.ingress_policy()
        && let Some(policy) = charlotte_launch::ingress_policy::decode(&entry.envelope)
    {
        return policy
            .assignments()
            .map(|binding| EffectiveIngressBinding {
                service: binding.service,
                backend_name: binding.backend_name.map(<[u8]>::to_vec),
            })
            .collect();
    }
    bootstrap
        .iter()
        .map(|binding| EffectiveIngressBinding {
            service: binding.service,
            backend_name: binding.backend_name.map(<[u8]>::to_vec),
        })
        .collect()
}

fn encode_effective_ingress_bindings(bindings: &[EffectiveIngressBinding]) -> Option<Vec<u8>> {
    let borrowed = bindings
        .iter()
        .map(|binding| charlotte_launch::ingress::ServiceBinding {
            service: binding.service,
            backend_name: binding.backend_name.as_deref(),
        })
        .collect::<Vec<_>>();
    let mut bytes = vec![0; charlotte_launch::ingress::encoded_len(&borrowed).ok()?];
    charlotte_launch::ingress::encode(&borrowed, &mut bytes).ok()?;
    Some(bytes)
}

/// Build the control-plane snapshot consumed by operator-facing adapters.
///
/// Every field comes from one locally applied Raft/catalog view. Dynamic
/// capacity samples retain an explicit freshness bit because only the leader
/// observes their receipt time; committed reservations remain useful even
/// when a sample has expired. Missing discovery routes suppress only the
/// derived ingress projection, never the underlying committed membership.
fn cluster_observability_snapshot(
    node: &RaftNode,
    transport: &RelmsgRaftTransport,
    catalog: &NameCatalog,
    local_mac: [u8; 6],
    ingress_services: &[charlotte_launch::ingress::ServiceBinding<'_>],
    capacity_last_seen_ms: &BTreeMap<u64, (u64, u64)>,
    controller: cluster_observe::ControllerCounters,
) -> Option<Vec<u8>> {
    if !node.can_serve_bounded_read(frouter::SNAPSHOT_SOURCE_MAX_AGE_MS) {
        return None;
    }

    let now_ms = node.millis();
    let self_key = node_identity::key_from_name(node.me.id.as_bytes())?;
    let leader_id = if node.state == NodeState::Leader {
        node.me.id.as_bytes().to_vec()
    } else {
        node.known_leader_id.as_deref().unwrap_or_default().as_bytes().to_vec()
    };
    let leader_key = node_identity::key_from_name(&leader_id);
    let mut flags = cluster_observe::FLAG_FRESH_COMMITTED;
    if node.state == NodeState::Leader {
        flags |= cluster_observe::FLAG_LOCAL_LEADER;
    }

    let deployments = catalog.deployments();
    let mut committed_frames = BTreeMap::<u64, u64>::new();
    for (_, deployment) in &deployments {
        let Some(descriptor) = charlotte_launch::deployment::decode(&deployment.descriptor) else {
            continue;
        };
        let demand = operations_admission::descriptor_memory_demand(&descriptor);
        for node_key in &deployment.replica_nodes {
            let total = committed_frames.entry(*node_key).or_default();
            *total = total.saturating_add(demand);
        }
    }

    let draining = catalog
        .ingress_draining_nodes()
        .into_iter()
        .map(|(node_key, _)| node_key)
        .collect::<BTreeSet<_>>();
    let active_members = node.cluster_configuration.active_voting_members();
    if active_members.len() > cluster_observe::MAX_NODES {
        flags |= cluster_observe::FLAG_TRUNCATED;
    }
    let mut nodes = active_members
        .into_iter()
        .filter_map(|peer| {
            let node_key = node_identity::key_from_name(peer.id.as_bytes())?;
            let mut node_flags = cluster_observe::NODE_MEMBER;
            if node_key == self_key {
                node_flags |= cluster_observe::NODE_SELF;
            }
            if leader_key == Some(node_key) {
                node_flags |= cluster_observe::NODE_LEADER;
            }
            if draining.contains(&node_key) {
                node_flags |= cluster_observe::NODE_DRAINING;
            }
            let mac = if node_key == self_key {
                Some(local_mac)
            } else {
                transport.mac_for_peer(&peer.id)
            }
            .filter(|mac| *mac != [0; 6]);
            if mac.is_some() {
                node_flags |= cluster_observe::NODE_MAC_PRESENT;
            }
            let capacity = catalog.node_capacity(node_key);
            if capacity.is_some() {
                node_flags |= cluster_observe::NODE_CAPACITY_PRESENT;
            }
            let capacity_fresh = capacity.is_some_and(|entry| {
                capacity_last_seen_ms.get(&node_key).is_some_and(|(nonce, seen_ms)| {
                    *nonce == entry.boot_nonce
                        && now_ms.saturating_sub(*seen_ms) <= CAPACITY_LEASE_MS
                })
            });
            if capacity_fresh {
                node_flags |= cluster_observe::NODE_CAPACITY_FRESH;
            }
            Some(cluster_observe::Node {
                node_key,
                flags: node_flags,
                mac: mac.unwrap_or([0; 6]),
                capacity_boot_nonce: capacity.map_or(0, |entry| entry.boot_nonce),
                capacity_epoch: capacity.map_or(0, |entry| entry.epoch),
                free_frames: capacity.map_or(0, |entry| entry.free_frames),
                usable_frames: capacity.map_or(0, |entry| entry.usable_frames),
                committed_frames: committed_frames.get(&node_key).copied().unwrap_or(0),
                cpu_load_permille: capacity.map_or(0, |entry| entry.cpu_load_permille),
            })
        })
        .collect::<Vec<_>>();
    nodes.sort_unstable_by_key(|entry| entry.node_key);
    nodes.truncate(cluster_observe::MAX_NODES);

    if deployments.len() > cluster_observe::MAX_DEPLOYMENTS {
        flags |= cluster_observe::FLAG_TRUNCATED;
    }
    let mut observed_deployments = Vec::new();
    for (name, deployment) in deployments.into_iter().take(cluster_observe::MAX_DEPLOYMENTS) {
        let placement = catalog.ingress_placement(&name);
        let service_generation = placement.as_ref().map_or(0, |view| view.service_generation);
        let mut ready_nodes = placement.map_or_else(Vec::new, |view| view.ready_nodes);
        let mut desired_nodes = deployment.replica_nodes;
        if desired_nodes.len() > cluster_observe::MAX_NODES
            || ready_nodes.len() > cluster_observe::MAX_NODES
        {
            flags |= cluster_observe::FLAG_TRUNCATED;
            desired_nodes.truncate(cluster_observe::MAX_NODES);
            ready_nodes.truncate(cluster_observe::MAX_NODES);
        }
        let demand_frames = charlotte_launch::deployment::decode(&deployment.descriptor)
            .map(|descriptor| operations_admission::descriptor_memory_demand(&descriptor))
            .unwrap_or(0);
        let state = if !ready_nodes.is_empty() && ready_nodes.len() == desired_nodes.len() {
            clusterctl::ROLLOUT_READY
        } else if ready_nodes.is_empty() {
            clusterctl::ROLLOUT_COMMITTED
        } else {
            clusterctl::ROLLOUT_REPLACING
        };
        observed_deployments.push(cluster_observe::Deployment {
            name,
            state,
            generation: deployment.generation,
            service_generation,
            object_id: deployment.object_id,
            demand_frames,
            desired_nodes,
            ready_nodes,
        });
    }

    let effective_ingress = effective_ingress_bindings(catalog, ingress_services);
    if effective_ingress.len() > cluster_observe::MAX_INGRESS {
        flags |= cluster_observe::FLAG_TRUNCATED;
    }
    let mut ingress = effective_ingress
        .into_iter()
        .take(cluster_observe::MAX_INGRESS)
        .map(|binding| {
            let projection = ingress_membership_snapshot(
                node,
                transport,
                catalog,
                local_mac,
                binding.backend_name.as_deref(),
            );
            cluster_observe::Ingress {
                service: binding.service,
                backend_name: binding.backend_name,
                projection_present: projection.is_some(),
                member_count: projection
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.members().len() as u16),
                advertiser_node: projection
                    .as_ref()
                    .and_then(BackendSnapshot::vip_advertiser)
                    .map(|backend| backend.node_id),
                epoch: projection.as_ref().map_or(0, |snapshot| snapshot.epoch),
                eligible_nodes: projection.map_or_else(Vec::new, |snapshot| {
                    snapshot.backends().iter().map(|backend| backend.node_id).collect()
                }),
            }
        })
        .collect::<Vec<_>>();
    ingress.sort_unstable_by_key(|entry| entry.service);

    let controller = if node.state == NodeState::Leader {
        controller
    } else {
        cluster_observe::ControllerCounters::default()
    };
    let state = match node.state {
        NodeState::Follower => 1,
        NodeState::Candidate => 2,
        NodeState::Leader => 3,
    };
    let mut snapshot = cluster_observe::Snapshot {
        flags,
        state,
        term: node.current_term,
        commit_index: node.commit_index,
        membership_epoch: node.membership_epoch(),
        observed_millis: now_ms,
        leader_id,
        self_id: node.me.id.as_bytes().to_vec(),
        controller,
        nodes,
        deployments: observed_deployments,
        ingress,
    };
    loop {
        if let Some(bytes) = cluster_observe::encode(&snapshot) {
            return Some(bytes);
        }
        // Records are already individually bounded and canonical. If the
        // aggregate exceeds 64 KiB, remove the largest collection's tail and
        // make the loss explicit instead of failing the whole keyhole.
        snapshot.flags |= cluster_observe::FLAG_TRUNCATED;
        snapshot.deployments.pop()?;
    }
}

fn reconcile_replica_placements(
    node: &mut RaftNode,
    catalog: &NameCatalog,
    pending: &mut Vec<PendingRegistration>,
    capacity: &operations_admission::NodeCapacityView,
    gates: &mut BTreeMap<Vec<u8>, operations_admission::ReassignmentGate>,
    reassignment_count: &mut u32,
    forced_reassignment_count: &mut u32,
) {
    if node.state != NodeState::Leader {
        return;
    }
    let candidates = placement_nodes(node, catalog);
    let automatic_node = node_identity::key_from_name(node.me.id.as_bytes()).unwrap_or(0);
    let deployments = catalog
        .deployments()
        .into_iter()
        .filter(|(_, entry)| {
            !entry.descriptor.is_empty()
                && charlotte_launch::deployment::decode(&entry.descriptor)
                    .is_some_and(|descriptor| descriptor.node_key == 0)
        })
        .collect::<Vec<_>>();
    let descriptors =
        deployments.iter().map(|(_, entry)| entry.descriptor.as_slice()).collect::<Vec<_>>();
    let Ok(assignments) = operations_admission::resolve_descriptor_assignments_with_capacity(
        &descriptors,
        &candidates,
        automatic_node,
        capacity,
    ) else {
        return;
    };
    let active_artifacts =
        deployments.iter().map(|(artifact, _)| artifact.clone()).collect::<BTreeSet<_>>();
    gates.retain(|artifact, _| active_artifacts.contains(artifact));
    for ((artifact, entry), nodes) in deployments.iter().zip(assignments) {
        if pending.len() >= MAX_IN_FLIGHT_CALLS {
            break;
        }
        if entry.replica_nodes == nodes
            || pending.iter().any(|pending| {
                matches!(
                    pending,
                    PendingRegistration::Placement {
                        artifact: pending_artifact,
                        ..
                    } if pending_artifact == artifact
                )
            })
        {
            continue;
        }
        let forced = entry.replica_nodes.iter().any(|node| !candidates.contains(node));
        let gate = gates.entry(artifact.clone()).or_default();
        if !gate.ready(
            &entry.replica_nodes,
            &nodes,
            forced,
            node.millis(),
            PLACEMENT_REASSIGN_DWELL_MS,
            PLACEMENT_REASSIGN_COOLDOWN_MS,
        ) {
            continue;
        }
        let Some(command) = encode_reassign(artifact, entry.generation, &nodes) else {
            continue;
        };
        if let Ok(log_index) = node.submit_command(command, node.millis()) {
            gate.mark_submitted(node.millis());
            *reassignment_count = reassignment_count.saturating_add(1);
            config::write_u32_release(dns::status::PLACEMENT_REASSIGNMENTS, *reassignment_count);
            if forced {
                *forced_reassignment_count = forced_reassignment_count.saturating_add(1);
                config::write_u32_release(
                    dns::status::FORCED_REASSIGNMENTS,
                    *forced_reassignment_count,
                );
            }
            catten_rt::logln!(
                "[dns] proposed {} reassignment for {}{}",
                if forced {
                    "topology"
                } else {
                    "pressure"
                },
                alloc::string::String::from_utf8_lossy(artifact),
                if forced {
                    " immediately"
                } else {
                    " after control dwell"
                }
            );
            pending.push(PendingRegistration::Placement {
                term: node.current_term,
                log_index,
                artifact: artifact.clone(),
            });
        }
    }
}

/// Drain the administration face of the DNS-owned Raft node.
fn drain_raft_admin(endpoint: u64, node: &mut RaftNode) {
    loop {
        let message = ipc_recv(endpoint);
        let _attachments =
            catten_services::RequestAttachments::new(message.memory, message.connection);
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

const CAPACITY_REPORT_INTERVAL_MS: u64 = 5_000;
const CAPACITY_LEASE_MS: u64 = CAPACITY_REPORT_INTERVAL_MS * 3;
const PLACEMENT_REASSIGN_DWELL_MS: u64 = 30_000;
const PLACEMENT_REASSIGN_COOLDOWN_MS: u64 = 60_000;

fn wait_capacity_call(mut call: PendingCall<'_>) -> Option<CallResult> {
    let (started, frequency_hz) = catten_syscall::monotonic_clock();
    loop {
        match call.poll() {
            Ok(Some(reply)) => return Some(reply),
            Ok(None) => {}
            Err(_) => return None,
        }
        let now = catten_syscall::monotonic_clock().0;
        if now.saturating_sub(started).saturating_mul(1_000) / frequency_hz.max(1) >= 100 {
            return None;
        }
        catten_services::sleep_ms(5);
    }
}

fn obtain_capacity_boot_nonce(ns_connection: ConnectionRef<'_>) -> Option<u64> {
    if let Some(nonce) = catten_syscall::random_u64().filter(|nonce| *nonce != 0) {
        return Some(nonce);
    }
    let lookup = ns_connection.call(ns::OP_TRY_LOOKUP, entropy::NAME).ok()?;
    let entropy = wait_capacity_call(lookup)?.connection?;
    let fill = entropy.as_ref().call(entropy::OP_FILL, 8).ok()?;
    let reply = wait_capacity_call(fill)?;
    if reply.result != 8 {
        return None;
    }
    let mapping = reply.memory?.map_read_only().ok()?;
    let nonce = u64::from_le_bytes(mapping.as_slice().get(..8)?.try_into().ok()?);
    (nonce != 0).then_some(nonce)
}

fn fresh_capacity_view(
    catalog: &NameCatalog,
    last_seen_ms: &BTreeMap<u64, (u64, u64)>,
    now_ms: u64,
) -> operations_admission::NodeCapacityView {
    let mut view = catalog.node_capacity_view();
    view.retain(|node, _| {
        let Some(entry) = catalog.node_capacity(*node) else {
            return false;
        };
        last_seen_ms.get(node).is_some_and(|(boot_nonce, seen)| {
            entry.boot_nonce == *boot_nonce && now_ms.saturating_sub(*seen) <= CAPACITY_LEASE_MS
        })
    });
    view
}

fn filter_capacity_report(
    filters: &mut BTreeMap<u64, operations_admission::CapacityFilter>,
    report: catten_services::rcapacity::Report,
) -> Option<(catten_services::rcapacity::Report, bool)> {
    let observed = operations_admission::NodeCapacity {
        free_frames: report.free_frames,
        usable_frames: report.usable_frames,
        committed_frames: 0,
        cpu_load_permille: report.cpu_load_permille,
    };
    let (filtered, seeded) = match filters.entry(report.node_key) {
        Entry::Occupied(mut entry) => {
            (entry.get_mut().observe(report.boot_nonce, report.epoch, observed)?, false)
        }
        Entry::Vacant(entry) => {
            let filter = operations_admission::CapacityFilter::new(
                report.boot_nonce,
                report.epoch,
                observed,
            )?;
            let filtered = filter.value();
            entry.insert(filter);
            (filtered, true)
        }
    };
    Some((
        catten_services::rcapacity::Report {
            free_frames: filtered.free_frames,
            usable_frames: filtered.usable_frames,
            cpu_load_permille: filtered.cpu_load_permille,
            ..report
        },
        seeded,
    ))
}

/// Combine fresh control samples with the resource promises held by all
/// previously committed deployments except artifacts replaced by this plan.
fn capacity_with_reservations(
    catalog: &NameCatalog,
    mut capacity: operations_admission::NodeCapacityView,
    replacing: &BTreeSet<Vec<u8>>,
) -> operations_admission::NodeCapacityView {
    for (artifact, entry) in catalog.deployments() {
        if replacing.contains(&artifact) {
            continue;
        }
        let Some(descriptor) = charlotte_launch::deployment::decode(&entry.descriptor) else {
            continue;
        };
        let demand = operations_admission::descriptor_memory_demand(&descriptor);
        for node in entry.replica_nodes {
            if let Some(sample) = capacity.get_mut(&node) {
                sample.committed_frames = sample.committed_frames.saturating_add(demand);
            } else {
                // Preserve already committed promises even while the node's
                // dynamic report is stale. Zero dynamic headroom prevents a
                // later release from treating that node as wholly unknown and
                // spending the same reservation again.
                capacity.insert(
                    node,
                    operations_admission::NodeCapacity {
                        free_frames: 0,
                        usable_frames: demand,
                        committed_frames: demand,
                        cpu_load_permille: operations_admission::CPU_LOAD_UNKNOWN,
                    },
                );
            }
        }
    }
    capacity
}

fn release_replacements(envelope: &[u8]) -> BTreeSet<Vec<u8>> {
    charlotte_launch::release::decode(envelope).map_or_else(BTreeSet::new, |release| {
        release
            .descriptors()
            .filter_map(charlotte_launch::deployment::decode)
            .map(|descriptor| descriptor.artifact_name.to_vec())
            .collect()
    })
}

fn release_capacity_view(
    catalog: &NameCatalog,
    capacity: operations_admission::NodeCapacityView,
    envelope: &[u8],
) -> operations_admission::NodeCapacityView {
    capacity_with_reservations(catalog, capacity, &release_replacements(envelope))
}

fn operations_capacity_view(
    catalog: &NameCatalog,
    capacity: operations_admission::NodeCapacityView,
    bundle: &[u8],
) -> operations_admission::NodeCapacityView {
    let replacing = charlotte_launch::operations_bundle::decode(bundle)
        .map_or_else(BTreeSet::new, |bundle| release_replacements(bundle.release));
    capacity_with_reservations(catalog, capacity, &replacing)
}

/// Build a committed capacity command when a report changes the placement
/// picture.
///
/// Workloads drift continuously, so the committed table would grow without
/// bound if every sample were replicated; `capacity_sample_changed` filters
/// unchanged samples. Placement decisions still read only applied state.
fn capacity_command(
    catalog: &NameCatalog,
    report: catten_services::rcapacity::Report,
) -> Option<Vec<u8>> {
    let next = operations_admission::NodeCapacity {
        free_frames: report.free_frames,
        usable_frames: report.usable_frames,
        committed_frames: 0,
        cpu_load_permille: report.cpu_load_permille,
    };
    let previous_entry = catalog.node_capacity(report.node_key);
    let previous = previous_entry.map(|entry| operations_admission::NodeCapacity {
        free_frames: entry.free_frames,
        usable_frames: entry.usable_frames,
        committed_frames: 0,
        cpu_load_permille: entry.cpu_load_permille,
    });
    let new_incarnation = previous_entry.is_some_and(|entry| entry.boot_nonce != report.boot_nonce);
    (new_incarnation || operations_admission::capacity_sample_changed(previous.as_ref(), &next))
        .then(|| {
            encode_node_capacity(&NodeCapacityEntry {
                node_key: report.node_key,
                boot_nonce: report.boot_nonce,
                epoch: report.epoch,
                free_frames: report.free_frames,
                usable_frames: report.usable_frames,
                cpu_load_permille: report.cpu_load_permille,
            })
        })
}

fn placement_nodes(node: &RaftNode, catalog: &NameCatalog) -> Vec<u64> {
    let draining =
        catalog.ingress_draining_nodes().into_iter().map(|(node, _)| node).collect::<BTreeSet<_>>();
    let mut nodes = node
        .cluster_configuration
        .active_voting_members()
        .iter()
        .filter_map(|peer| node_identity::key_from_name(peer.id.as_bytes()))
        .filter(|node| !draining.contains(node))
        .collect::<Vec<_>>();
    nodes.sort_unstable();
    nodes.dedup();
    nodes
}

fn release_command(
    envelope: &[u8],
    eligible_nodes: &[u64],
    automatic_node: u64,
    capacity: &operations_admission::NodeCapacityView,
) -> Result<Vec<u8>, i64> {
    let assignments = operations_admission::resolve_release_assignments_with_capacity(
        envelope,
        eligible_nodes,
        automatic_node,
        capacity,
    )
    .map_err(|error| match error {
        operations_admission::AdmissionError::UnsatisfiablePlacement => {
            clusterctl::ERR_UNSATISFIABLE_PLACEMENT
        }
        operations_admission::AdmissionError::InsufficientCapacity => {
            clusterctl::ERR_INSUFFICIENT_CAPACITY
        }
        _ => clusterctl::ERR_UNTRUSTED_DESCRIPTOR,
    })?;
    catten_services::name_catalog::encode_release_replicas(envelope, &assignments)
        .ok_or(dns::ERR_TOO_LARGE)
}

/// Bound on the synchronous time-service query. The DNS reactor owns Raft
/// heartbeats, so a wedged time service must fail the operation rather than
/// stall consensus indefinitely.
const TIME_QUERY_TIMEOUT_MS: u64 = 100;

fn trusted_unix_seconds(time: ConnectionRef<'_>) -> Option<u64> {
    let mut call = time.call(catten_services::time::OP_UNIX_SECONDS, 0).ok()?;
    let (start_ticks, frequency_hz) = catten_syscall::monotonic_clock();
    let frequency_hz = frequency_hz.max(1);
    loop {
        match call.poll() {
            Ok(Some(reply)) => return u64::try_from(reply.result).ok(),
            Ok(None) => {}
            Err(_) => return None,
        }
        let (now_ticks, _) = catten_syscall::monotonic_clock();
        let elapsed_ms = now_ticks.saturating_sub(start_ticks).saturating_mul(1000) / frequency_hz;
        if elapsed_ms >= TIME_QUERY_TIMEOUT_MS {
            return None;
        }
        catten_services::sleep_ms(5);
    }
}

fn operations_command(
    bundle: &[u8],
    trust: &charlotte_launch::trust::AdmissionTrust,
    time: ConnectionRef<'_>,
    eligible_nodes: &[u64],
    automatic_node: u64,
    capacity: &operations_admission::NodeCapacityView,
) -> Result<Vec<u8>, i64> {
    let now = trusted_unix_seconds(time).ok_or(clusterctl::ERR_TIME_UNAVAILABLE)?;
    operations_admission::verify_and_encode_with_capacity(
        bundle,
        trust,
        now,
        eligible_nodes,
        automatic_node,
        capacity,
    )
    .map_err(|error| match error {
        operations_admission::AdmissionError::Expired => clusterctl::ERR_EXPIRED_OPERATION,
        operations_admission::AdmissionError::TooLarge => clusterctl::ERR_TOO_LARGE,
        operations_admission::AdmissionError::UnsatisfiablePlacement => {
            clusterctl::ERR_UNSATISFIABLE_PLACEMENT
        }
        operations_admission::AdmissionError::InsufficientCapacity => {
            clusterctl::ERR_INSUFFICIENT_CAPACITY
        }
        operations_admission::AdmissionError::Invalid
        | operations_admission::AdmissionError::WrongCluster
        | operations_admission::AdmissionError::WrongOperationsKey
        | operations_admission::AdmissionError::WrongRecipient
        | operations_admission::AdmissionError::WrongReleaseKey => {
            clusterctl::ERR_UNTRUSTED_DESCRIPTOR
        }
    })
}

fn shutdown_command(
    envelope: &[u8],
    catalog: &NameCatalog,
    trust: &charlotte_launch::trust::AdmissionTrust,
    time: ConnectionRef<'_>,
) -> Result<Vec<u8>, i64> {
    let key = catalog.cluster_key().unwrap_or(trust.deployment_key);
    if charlotte_launch::shutdown::verify(envelope, &key)
        != charlotte_launch::shutdown::VerifyOutcome::Valid
    {
        return Err(clusterctl::ERR_UNTRUSTED_DESCRIPTOR);
    }
    let fields =
        charlotte_launch::shutdown::decode(envelope).ok_or(clusterctl::ERR_UNTRUSTED_DESCRIPTOR)?;
    let now = trusted_unix_seconds(time).ok_or(clusterctl::ERR_TIME_UNAVAILABLE)?;
    if now < fields.not_before_unix_seconds || now > fields.expires_unix_seconds {
        return Err(clusterctl::ERR_OUTSIDE_VALIDITY);
    }
    encode_shutdown(envelope).ok_or(clusterctl::ERR_TOO_LARGE)
}

fn ingress_policy_command(
    envelope: &[u8],
    trust: &charlotte_launch::trust::AdmissionTrust,
    time: ConnectionRef<'_>,
) -> Result<Vec<u8>, i64> {
    match charlotte_launch::ingress_policy::verify(
        envelope,
        &trust.cluster_id,
        &trust.operations_key,
    ) {
        charlotte_launch::ingress_policy::VerifyOutcome::Valid => {}
        charlotte_launch::ingress_policy::VerifyOutcome::Invalid
        | charlotte_launch::ingress_policy::VerifyOutcome::WrongCluster
        | charlotte_launch::ingress_policy::VerifyOutcome::WrongKey => {
            return Err(clusterctl::ERR_UNTRUSTED_DESCRIPTOR);
        }
    }
    let policy = charlotte_launch::ingress_policy::decode(envelope)
        .ok_or(clusterctl::ERR_UNTRUSTED_DESCRIPTOR)?;
    let now = trusted_unix_seconds(time).ok_or(clusterctl::ERR_TIME_UNAVAILABLE)?;
    if now < policy.not_before_unix_seconds || now > policy.expires_unix_seconds {
        return Err(clusterctl::ERR_OUTSIDE_VALIDITY);
    }
    encode_ingress_policy(envelope).ok_or(clusterctl::ERR_TOO_LARGE)
}

/// Fail one deferred admission whose command can no longer be trusted to
/// occupy the log index it captured (term changed) or that exceeded the
/// pending-queue bound. Callers observe an uncertain outcome and may retry
/// idempotently.
fn abort_pending_entry(entry: PendingRegistration, transport: &RelmsgRaftTransport) {
    match entry {
        PendingRegistration::Prepare {
            reply,
            connection,
            ..
        }
        | PendingRegistration::Activate {
            reply,
            connection,
            ..
        } => {
            if connection != 0 {
                ipc_close(connection);
            }
            if reply != 0 {
                ipc_reply(reply, dns::ERR_UNCERTAIN);
            }
        }
        PendingRegistration::RemoteRegister {
            reply,
            connection,
            ..
        } => {
            if connection != 0 {
                ipc_close(connection);
            }
            if reply != 0 {
                ipc_reply(reply, dns::ERR_UNCERTAIN);
            }
        }
        PendingRegistration::Unregister {
            reply,
            ..
        }
        | PendingRegistration::Deploy {
            reply,
            ..
        }
        | PendingRegistration::SetKey {
            reply,
            ..
        } => {
            if reply != 0 {
                ipc_reply(reply, dns::ERR_UNCERTAIN);
            }
        }
        PendingRegistration::Placement {
            ..
        } => {}
        PendingRegistration::RemoteDeploy {
            peer,
            session,
            request_id,
            ..
        } => {
            transport.send_message(
                &peer,
                catten_services::rdeploy::TAG_REPLY,
                catten_services::rdeploy::encode_reply(session, request_id, dns::ERR_UNCERTAIN),
            );
        }
        PendingRegistration::RemoteRelease {
            peer,
            session,
            request_id,
            ..
        } => {
            transport.send_message(
                &peer,
                catten_services::rrelease::TAG_REPLY,
                catten_services::rrelease::encode_reply(session, request_id, dns::ERR_UNCERTAIN),
            );
        }
        PendingRegistration::RemoteOperations {
            peer,
            session,
            request_id,
            ..
        } => {
            transport.send_message(
                &peer,
                catten_services::roperations::TAG_REPLY,
                catten_services::roperations::encode_reply(session, request_id, dns::ERR_UNCERTAIN),
            );
        }
        PendingRegistration::RemoteShutdown {
            peer,
            session,
            request_id,
            ..
        } => {
            transport.send_message(
                &peer,
                catten_services::rshutdown::TAG_REPLY,
                catten_services::rshutdown::encode_reply(session, request_id, dns::ERR_UNCERTAIN),
            );
        }
        PendingRegistration::RemoteIngressPolicy {
            peer,
            session,
            request_id,
            ..
        } => {
            transport.send_message(
                &peer,
                catten_services::ringress_policy::TAG_REPLY,
                catten_services::ringress_policy::encode_reply(
                    session,
                    request_id,
                    dns::ERR_UNCERTAIN,
                ),
            );
        }
        PendingRegistration::RemotePrepare {
            name,
            owner,
            ..
        }
        | PendingRegistration::RemoteActivate {
            name,
            owner,
            ..
        } => {
            let reply = catten_services::rregister::encode_reply(&owner, &name, 0);
            let owner = alloc::string::String::from_utf8_lossy(&owner);
            transport.send_message(&owner, catten_services::rregister::TAG_REPLY, reply);
        }
    }
}

/// Relay one already-decoded admission request to the current leader.
///
/// Returns `Some(code)` when the caller must reply with `code`, or `None`
/// once the request is queued and its reply deferred to the reactor.
#[allow(clippy::too_many_arguments)]
fn relay_to_leader(
    node: &RaftNode,
    transport: &RelmsgRaftTransport,
    pending_queries: &mut Vec<PendingQuery>,
    next_query_id: &mut u64,
    session: u64,
    reply: u64,
    timeout_ms: u64,
    tag: u8,
    encode: impl FnOnce(u64, u64) -> Option<Vec<u8>>,
    kind: impl FnOnce(u64) -> PendingQueryKind,
) -> Option<i64> {
    let Some(leader) = node.known_leader_id.clone() else {
        return Some(dns::ERR_NOT_LEADER);
    };
    if pending_queries.len() >= MAX_IN_FLIGHT_CALLS || !transport.has_peer(&leader) {
        return Some(dns::ERR_BUSY);
    }
    let request_id = *next_query_id;
    *next_query_id = next_query_id.wrapping_add(1).max(1);
    let Some(frame) = encode(session, request_id) else {
        return Some(dns::ERR_TOO_LARGE);
    };
    pending_queries.push(PendingQuery {
        query_id: request_id,
        expected_leader: leader.clone(),
        deadline: node.millis().saturating_add(timeout_ms),
        kind: kind(reply),
    });
    transport.send_message(&leader, tag, frame);
    None
}

/// Submit one admission command locally and record it for completion when it
/// commits. Returns `Ok(())` when the caller must `continue` (the reply is
/// deferred to the reactor), or `Err(code)` when the caller must reply.
fn submit_deferred(
    node: &mut RaftNode,
    pending: &mut Vec<PendingRegistration>,
    reply: u64,
    build: impl FnOnce(&RaftNode) -> Result<Vec<u8>, i64>,
) -> Result<(), i64> {
    let command = build(node)?;
    match node.submit_command(command, node.millis()) {
        Ok(log_index) => {
            pending.push(PendingRegistration::Deploy {
                term: node.current_term,
                log_index,
                reply,
            });
            Ok(())
        }
        Err(code) => Err(code),
    }
}

fn serve(ctx: &Context) -> ShutdownRequest {
    config::write_u32_release(dns::status::STAGE, 1);
    let mnemonic: Vec<u8> = match ctx.manifest_value(CLUSTER_KEY) {
        Some(ManifestValue::Bytes(raw)) if !raw.is_empty() => raw.to_vec(),
        _ => b"charlotte".to_vec(),
    };
    let election_timeout_ms = match ctx.manifest_value(ELECTION_KEY) {
        Some(ManifestValue::Unsigned(value)) => value,
        _ => 300,
    };
    let ingress_services = match ctx.manifest_value(INGRESS_SERVICES_KEY) {
        Some(ManifestValue::Bytes(bytes)) => charlotte_launch::ingress::decode(bytes)
            .unwrap_or_else(|| fatal(23))
            .collect::<Vec<_>>(),
        Some(_) => fatal(23),
        None => Vec::new(),
    };
    let admission_trust = match ctx.manifest_value(charlotte_launch::ADMISSION_TRUST_MANIFEST_KEY) {
        Some(ManifestValue::Bytes(bytes)) => {
            charlotte_launch::trust::AdmissionTrust::decode(bytes).unwrap_or_else(|| fatal(21))
        }
        _ => fatal(21),
    };
    // Keep heartbeats comfortably below the election timeout without
    // flooding the serialized relmsg path on a slow emulator.
    let heartbeat_interval_ms = (election_timeout_ms / 4).clamp(25, 500);

    let names = match ctx.bootstrap_connection() {
        Some(connection) => connection,
        None => fatal(1),
    };
    let ns_conn = names.as_raw();
    config::write_u32_release(dns::status::STAGE, 2);

    // MAC and persisted node identity.
    let net_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, net::NAME);
    if net_lookup == 0 {
        fatal(2);
    }
    let (net_generation, net_conn) = unsafe { wait_reply(net_lookup) };
    if net_generation < 1 || net_conn == 0 {
        fatal(3);
    }
    let status_call = ipc_scalar_call(net_conn, net::OP_STATUS, 0);
    if status_call == 0 {
        fatal(4);
    }
    let (status, _) = unsafe { wait_reply(status_call) };
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
    if let Err(request) = wait_for_local_ready_or_shutdown(ctx, names) {
        return request;
    }
    config::write_u32_release(dns::status::STAGE, 4);

    let relmsg_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, relmsg::NAME);
    if relmsg_lookup == 0 {
        fatal(8);
    }
    let (relmsg_generation, relmsg_conn) = unsafe { wait_reply(relmsg_lookup) };
    if relmsg_generation < 1 || relmsg_conn == 0 {
        fatal(9);
    }
    let disco_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, disco::NAME);
    if disco_lookup == 0 {
        fatal(10);
    }
    let (disco_generation, disco_conn) = unsafe { wait_reply(disco_lookup) };
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
    let (generation, _) = unsafe { wait_reply(register) };
    if generation < 1 {
        fatal(15);
    }
    let dns_session = generation as u64;
    // Operational admission must fail closed unless the existing UTC service
    // can evaluate signed expiry. Retain this looked-up connection as an
    // owner for the DNS process lifetime.
    let (_, time_conn) = wait_for_registered_name_owned(
        ctx.bootstrap_connection().unwrap_or_else(|| fatal(22)),
        time::NAME,
    )
    .unwrap_or_else(|| fatal(22));

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
    let (raft_generation, _) = unsafe { wait_reply(raft_register) };
    if raft_generation < 1 {
        fatal(20);
    }
    config::write_u32_release(dns::status::STAGE, 6);

    // A fresh durable identity starts as a one-member cluster. Discovery only
    // supplies transient MAC routes; admission itself is a command in this
    // same durable Raft log.
    let transport = Arc::new(RelmsgRaftTransport::new(relmsg_conn));
    transport.set_net_send(net_conn, local_mac, catten_services::raft::ETHERTYPE);
    config::write_u32_release(dns::status::STAGE, 7);

    let me = Peer::voter(node_name_str.clone(), raft_name);
    config::write_u32_release(dns::status::PEER_COUNT, 1);

    let catalog = NameCatalog::new_with_control_plane_trust(
        admission_trust.deployment_key,
        admission_trust.operations_key,
        admission_trust.cluster_id,
    );
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
        // Bound the durable log with periodic local snapshots. The catalog
        // state machine snapshots the complete committed catalog, so the
        // threshold trades log memory and boot replay time against snapshot
        // serialization frequency.
        snapshot_min_entries: 1024,
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
    let mut membership_events_submitted: BTreeSet<Vec<u8>> = BTreeSet::new();
    let mut membership_event_term = 0u64;
    let mut logged_membership_epoch = u64::MAX;
    let mut logged_raft_term = u64::MAX;
    let mut logged_raft_state = NodeState::Follower;
    let mut next_joint_diagnostic_ms = 0u64;
    let mut timer_armed = submit_detached_timer(LOOP_TICK_MS, 0, RAFT_TIMER_COOKIE) != u64::MAX;
    let mut last_heartbeat_broadcast = 0u64;
    let mut capacity_boot_nonce = None;
    let mut capacity_last_seen_ms: BTreeMap<u64, (u64, u64)> = BTreeMap::new();
    let mut capacity_filters: BTreeMap<u64, operations_admission::CapacityFilter> = BTreeMap::new();
    let mut placement_gates: BTreeMap<Vec<u8>, operations_admission::ReassignmentGate> =
        BTreeMap::new();
    let mut capacity_reports_accepted = 0u32;
    let mut capacity_commands_proposed = 0u32;
    let mut placement_reassignments = 0u32;
    let mut forced_reassignments = 0u32;
    let mut next_capacity_report_ms = 0u64;

    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            if recv_pending != 0 {
                ipc_close(recv_pending);
            }
            for reply in event_waiters.drain() {
                ipc_reply(reply, clusterctl::ERR_NOT_LEADER);
            }
            catten_rt::logln!(
                "[dns] shutdown: cancelling {} register(s), {} call(s), {} query(s), and {} local \
                 call(s)",
                pending_registers.len(),
                in_flight_calls.len(),
                pending_queries.len(),
                pending_local_calls.len()
            );
            return request;
        }
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

        // Report local capacity. The leader proposes a committed sample when
        // the report changes the placement picture; followers relay reports
        // to the leader so every replica eventually resolves from applied
        // state. Unknown nodes stay neutral in the resolver.
        if node.millis() >= next_capacity_report_ms {
            next_capacity_report_ms = node.millis().saturating_add(CAPACITY_REPORT_INTERVAL_MS);
            if capacity_boot_nonce.is_none()
                && let Some(ns_connection) = ctx.bootstrap_connection()
            {
                capacity_boot_nonce = obtain_capacity_boot_nonce(ns_connection);
            }
            let (free_frames, usable_frames, cpu_load_permille) = catten_syscall::node_pressure();
            if let (Some(node_key), Some(boot_nonce)) =
                (node_identity::key_from_name(&node_name), capacity_boot_nonce)
            {
                let report = catten_services::rcapacity::Report {
                    node_key,
                    boot_nonce,
                    epoch: catten_syscall::monotonic_clock().0,
                    free_frames,
                    usable_frames,
                    cpu_load_permille: cpu_load_permille.min(u64::from(u16::MAX)) as u16,
                };
                if node.state == NodeState::Leader {
                    if let Some((control, seeded)) =
                        filter_capacity_report(&mut capacity_filters, report)
                    {
                        capacity_reports_accepted = capacity_reports_accepted.saturating_add(1);
                        config::write_u32_release(
                            dns::status::CAPACITY_REPORTS_ACCEPTED,
                            capacity_reports_accepted,
                        );
                        capacity_last_seen_ms
                            .insert(control.node_key, (control.boot_nonce, node.millis()));
                        if seeded {
                            catten_rt::logln!(
                                "[dns] seeded capacity control for node {:016x}",
                                control.node_key
                            );
                        }
                        if let Some(command) = capacity_command(&catalog, control)
                            && node.submit_command(command, node.millis()).is_ok()
                        {
                            capacity_commands_proposed =
                                capacity_commands_proposed.saturating_add(1);
                            config::write_u32_release(
                                dns::status::CAPACITY_COMMANDS_PROPOSED,
                                capacity_commands_proposed,
                            );
                            catten_rt::logln!(
                                "[dns] proposed filtered capacity for node {:016x}",
                                control.node_key
                            );
                        }
                    }
                } else if let Some(leader) = node.known_leader_id.clone()
                    && transport.has_peer(&leader)
                {
                    transport.send_message(
                        &leader,
                        catten_services::rcapacity::TAG_REQUEST,
                        catten_services::rcapacity::encode_request(report),
                    );
                }
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
                && !join_request_pending
                && node.millis() >= join_retry_at_ms
                && join_request_allowed(
                    node.cluster_configuration.all_members().len(),
                    node.state == NodeState::Leader,
                    node.joining && node.joining_from.as_deref() == Some(anchor.as_str()),
                )
                && let Some(payload) = encode_join_request(node.me.id.as_bytes(), raft_name)
            {
                if !node.joining {
                    node.begin_joining(anchor.clone(), node.millis());
                }
                catten_rt::logln!(
                    "[dns] JOIN REQUEST self={} anchor={} term={}",
                    node.me.id,
                    anchor,
                    node.current_term
                );
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
                    let capacity = memory_size(memory);
                    let (rx_scratch_map_status, rx_scratch_vaddr) = memory_map_any(memory, false);
                    if rx_scratch_map_status == 0 {
                        let object = unsafe {
                            core::slice::from_raw_parts(rx_scratch_vaddr as *const u8, capacity)
                        };
                        let Ok(envelope) = parse_ipc_message_header(object) else {
                            memory_unmap(memory);
                            memory_close(memory);
                            continue;
                        };
                        let source_mac = envelope.peer;
                        let len = envelope.payload_len as usize;
                        if result != envelope.payload_len as u64 {
                            memory_unmap(memory);
                            memory_close(memory);
                            continue;
                        }
                        let frame = &object[IPC_MESSAGE_HEADER_SIZE..IPC_MESSAGE_HEADER_SIZE + len];
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
                                                        let destination =
                                                            LocalCallDestination::Remote {
                                                                caller: caller.clone(),
                                                                session,
                                                                call_id,
                                                                target_generation,
                                                                peer: source_peer.clone(),
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
                                            // Reserve the reply ordinal at the point the
                                            // reply is actually sent, so the per-peer ACK
                                            // count corresponds to send order even when
                                            // concurrently admitted calls complete out of
                                            // order.
                                            let reply_ordinal = next_reply_ordinal
                                                .entry(source_peer.clone())
                                                .or_insert_with(|| {
                                                    transport.acknowledged_count_for(
                                                        &source_peer,
                                                        catten_services::rcall::TAG_REPLY,
                                                    )
                                                });
                                            *reply_ordinal = reply_ordinal.saturating_add(1);
                                            let settled_after_ack = *reply_ordinal;
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
                                                    | PendingQueryKind::Release { .. }
                                                    | PendingQueryKind::Operations { .. }
                                                    | PendingQueryKind::Shutdown { .. }
                                                    | PendingQueryKind::IngressPolicy { .. }
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
                                        }
                                        | PendingQueryKind::Release {
                                            reply,
                                        }
                                        | PendingQueryKind::Operations {
                                            reply,
                                        }
                                        | PendingQueryKind::Shutdown {
                                            reply,
                                        }
                                        | PendingQueryKind::IngressPolicy {
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
                                    && catalog.lookup_owner(&name, &owner).is_some_and(|entry| {
                                        entry.generation == generation
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
                                    pending_registers.push(PendingRegistration::Unregister { term: node.current_term,
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
                                    pending_registers.push(PendingRegistration::RemotePrepare { term: node.current_term,
                                        log_index,
                                        name,
                                        owner,
                                    });
                                }
                            }
                            Some(catten_services::rcapacity::TAG_REQUEST) => {
                                // Capacity from a committed member; the sender
                                // must own the node key it claims. The leader
                                // commits a sample only when it changes the
                                // placement picture.
                                if node.state == NodeState::Leader
                                    && let Some(report) =
                                        catten_services::rcapacity::decode_request(frame)
                                    && transport.peer_id_for_mac(&source_mac).is_some_and(
                                        |peer| {
                                            node_identity::key_from_name(peer.as_bytes())
                                                == Some(report.node_key)
                                        },
                                    )
                                    && let Some((control, seeded)) =
                                        filter_capacity_report(&mut capacity_filters, report)
                                {
                                    capacity_reports_accepted =
                                        capacity_reports_accepted.saturating_add(1);
                                    config::write_u32_release(
                                        dns::status::CAPACITY_REPORTS_ACCEPTED,
                                        capacity_reports_accepted,
                                    );
                                    capacity_last_seen_ms.insert(
                                        control.node_key,
                                        (control.boot_nonce, node.millis()),
                                    );
                                    if seeded {
                                        catten_rt::logln!(
                                            "[dns] seeded capacity control for node {:016x}",
                                            control.node_key
                                        );
                                    }
                                    if let Some(command) = capacity_command(&catalog, control)
                                        && node.submit_command(command, node.millis()).is_ok()
                                    {
                                        capacity_commands_proposed =
                                            capacity_commands_proposed.saturating_add(1);
                                        config::write_u32_release(
                                            dns::status::CAPACITY_COMMANDS_PROPOSED,
                                            capacity_commands_proposed,
                                        );
                                        catten_rt::logln!(
                                            "[dns] proposed filtered capacity for node {:016x}",
                                            control.node_key
                                        );
                                    }
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
                                                pending_registers.push(PendingRegistration::RemoteDeploy { term: node.current_term,
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
                            Some(catten_services::rrelease::TAG_REQUEST) => {
                                if let Some(request) =
                                    catten_services::rrelease::decode_request(frame)
                                    && let Some(source_peer) =
                                        transport.peer_id_for_mac(&source_mac)
                                    && source_peer.as_bytes() == request.caller
                                {
                                    let result = if node.state != NodeState::Leader {
                                        Some(dns::ERR_NOT_LEADER)
                                    } else {
                                        let automatic_node = node_identity::key_from_name(&node_name)
                                            .unwrap_or(0);
                                        let eligible_nodes = placement_nodes(&node, &catalog);
                                        let capacity = release_capacity_view(
                                            &catalog,
                                            fresh_capacity_view(
                                                &catalog,
                                                &capacity_last_seen_ms,
                                                node.millis(),
                                            ),
                                            &request.envelope,
                                        );
                                        match release_command(
                                            &request.envelope,
                                            &eligible_nodes,
                                            automatic_node,
                                            &capacity,
                                        ) {
                                            Ok(command) => match node
                                                .submit_command(command, node.millis())
                                            {
                                                Ok(log_index) => {
                                                    pending_registers.push(PendingRegistration::RemoteRelease { term: node.current_term,
                                                            log_index,
                                                            peer: source_peer.clone(),
                                                            session: request.session,
                                                            request_id: request.request_id,
                                                        },
                                                    );
                                                    None
                                                }
                                                Err(code) => Some(code),
                                            },
                                            Err(code) => Some(code),
                                        }
                                    };
                                    if let Some(result) = result {
                                        transport.send_message(
                                            &source_peer,
                                            catten_services::rrelease::TAG_REPLY,
                                            catten_services::rrelease::encode_reply(
                                                request.session,
                                                request.request_id,
                                                result,
                                            ),
                                        );
                                    }
                                }
                            }
                            Some(catten_services::rrelease::TAG_REPLY) => {
                                if let Some((session, request_id, result)) =
                                    catten_services::rrelease::decode_reply(frame)
                                    && session == dns_session
                                    && let Some(source_peer) =
                                        transport.peer_id_for_mac(&source_mac)
                                    && let Some(index) =
                                        pending_queries.iter().position(|query| {
                                            query.query_id == request_id
                                                && query.expected_leader == source_peer
                                                && matches!(
                                                    query.kind,
                                                    PendingQueryKind::Release { .. }
                                                )
                                        })
                                {
                                    let query = pending_queries.swap_remove(index);
                                    let PendingQueryKind::Release {
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
                            Some(catten_services::roperations::TAG_REQUEST) => {
                                if let Some(request) =
                                    catten_services::roperations::decode_request(frame)
                                    && let Some(source_peer) =
                                        transport.peer_id_for_mac(&source_mac)
                                    && source_peer.as_bytes() == request.caller
                                {
                                    let result = if node.state != NodeState::Leader {
                                        Some(dns::ERR_NOT_LEADER)
                                    } else {
                                        let automatic_node = node_identity::key_from_name(&node_name)
                                            .unwrap_or(0);
                                        let eligible_nodes = placement_nodes(&node, &catalog);
                                        let capacity = operations_capacity_view(
                                            &catalog,
                                            fresh_capacity_view(
                                                &catalog,
                                                &capacity_last_seen_ms,
                                                node.millis(),
                                            ),
                                            &request.bundle,
                                        );
                                        match operations_command(
                                            &request.bundle,
                                            &admission_trust,
                                            time_conn.as_ref(),
                                            &eligible_nodes,
                                            automatic_node,
                                            &capacity,
                                        ) {
                                            Ok(command) => match node
                                                .submit_command(command, node.millis())
                                            {
                                                Ok(log_index) => {
                                                    pending_registers.push(PendingRegistration::RemoteOperations { term: node.current_term,
                                                            log_index,
                                                            peer: source_peer.clone(),
                                                            session: request.session,
                                                            request_id: request.request_id,
                                                        },
                                                    );
                                                    None
                                                }
                                                Err(code) => Some(code),
                                            },
                                            Err(code) => Some(code),
                                        }
                                    };
                                    if let Some(result) = result {
                                        transport.send_message(
                                            &source_peer,
                                            catten_services::roperations::TAG_REPLY,
                                            catten_services::roperations::encode_reply(
                                                request.session,
                                                request.request_id,
                                                result,
                                            ),
                                        );
                                    }
                                }
                            }
                            Some(catten_services::roperations::TAG_REPLY) => {
                                if let Some((session, request_id, result)) =
                                    catten_services::roperations::decode_reply(frame)
                                    && session == dns_session
                                    && let Some(source_peer) =
                                        transport.peer_id_for_mac(&source_mac)
                                    && let Some(index) =
                                        pending_queries.iter().position(|query| {
                                            query.query_id == request_id
                                                && query.expected_leader == source_peer
                                                && matches!(
                                                    query.kind,
                                                    PendingQueryKind::Operations { .. }
                                                )
                                        })
                                {
                                    let query = pending_queries.swap_remove(index);
                                    let PendingQueryKind::Operations {
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
                            Some(catten_services::rshutdown::TAG_REQUEST) => {
                                if let Some(request) =
                                    catten_services::rshutdown::decode_request(frame)
                                    && let Some(source_peer) =
                                        transport.peer_id_for_mac(&source_mac)
                                    && source_peer.as_bytes() == request.caller
                                {
                                    let result = if node.state != NodeState::Leader {
                                        Some(dns::ERR_NOT_LEADER)
                                    } else {
                                        match shutdown_command(
                                            &request.envelope,
                                            &catalog,
                                            &admission_trust,
                                            time_conn.as_ref(),
                                        ) {
                                            Ok(command) => match node
                                                .submit_command(command, node.millis())
                                            {
                                                Ok(log_index) => {
                                                    pending_registers.push(PendingRegistration::RemoteShutdown { term: node.current_term,
                                                            log_index,
                                                            peer: source_peer.clone(),
                                                            session: request.session,
                                                            request_id: request.request_id,
                                                        },
                                                    );
                                                    None
                                                }
                                                Err(code) => Some(code),
                                            },
                                            Err(code) => Some(code),
                                        }
                                    };
                                    if let Some(result) = result {
                                        transport.send_message(
                                            &source_peer,
                                            catten_services::rshutdown::TAG_REPLY,
                                            catten_services::rshutdown::encode_reply(
                                                request.session,
                                                request.request_id,
                                                result,
                                            ),
                                        );
                                    }
                                }
                            }
                            Some(catten_services::rshutdown::TAG_REPLY) => {
                                if let Some((session, request_id, result)) =
                                    catten_services::rshutdown::decode_reply(frame)
                                    && session == dns_session
                                    && let Some(source_peer) =
                                        transport.peer_id_for_mac(&source_mac)
                                    && let Some(index) =
                                        pending_queries.iter().position(|query| {
                                            query.query_id == request_id
                                                && query.expected_leader == source_peer
                                                && matches!(
                                                    query.kind,
                                                    PendingQueryKind::Shutdown { .. }
                                                )
                                        })
                                {
                                    let query = pending_queries.swap_remove(index);
                                    let PendingQueryKind::Shutdown {
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
                            Some(catten_services::ringress_policy::TAG_REQUEST) => {
                                if let Some(request) =
                                    catten_services::ringress_policy::decode_request(frame)
                                    && let Some(source_peer) =
                                        transport.peer_id_for_mac(&source_mac)
                                    && source_peer.as_bytes() == request.caller
                                {
                                    let result = if node.state != NodeState::Leader {
                                        Some(dns::ERR_NOT_LEADER)
                                    } else {
                                        match ingress_policy_command(
                                            &request.envelope,
                                            &admission_trust,
                                            time_conn.as_ref(),
                                        ) {
                                            Ok(command) => match node
                                                .submit_command(command, node.millis())
                                            {
                                                Ok(log_index) => {
                                                    pending_registers.push(PendingRegistration::RemoteIngressPolicy { term: node.current_term,
                                                            log_index,
                                                            peer: source_peer.clone(),
                                                            session: request.session,
                                                            request_id: request.request_id,
                                                        },
                                                    );
                                                    None
                                                }
                                                Err(code) => Some(code),
                                            },
                                            Err(code) => Some(code),
                                        }
                                    };
                                    if let Some(result) = result {
                                        transport.send_message(
                                            &source_peer,
                                            catten_services::ringress_policy::TAG_REPLY,
                                            catten_services::ringress_policy::encode_reply(
                                                request.session,
                                                request.request_id,
                                                result,
                                            ),
                                        );
                                    }
                                }
                            }
                            Some(catten_services::ringress_policy::TAG_REPLY) => {
                                if let Some((session, request_id, result)) =
                                    catten_services::ringress_policy::decode_reply(frame)
                                    && session == dns_session
                                    && let Some(source_peer) =
                                        transport.peer_id_for_mac(&source_mac)
                                    && let Some(index) =
                                        pending_queries.iter().position(|query| {
                                            query.query_id == request_id
                                                && query.expected_leader == source_peer
                                                && matches!(
                                                    query.kind,
                                                    PendingQueryKind::IngressPolicy { .. }
                                                )
                                        })
                                {
                                    let query = pending_queries.swap_remove(index);
                                    let PendingQueryKind::IngressPolicy {
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
                                let mut response_mac = source_mac;
                                let accepted = decode_join_request(&frame[1..])
                                    .and_then(|(joiner_id, service_name)| {
                                        let joiner = core::str::from_utf8(joiner_id).ok()?;
                                        let source_peer = transport.peer_id_for_mac(&source_mac)?;
                                        let direct = source_peer == joiner;
                                        if node.state != NodeState::Leader {
                                            // A deterministic admission anchor can cease being
                                            // leader while the request is in flight. Relay only a
                                            // directly sourced request and only to the leader this
                                            // committed member currently follows.
                                            if direct
                                                && let Some(leader) = node.known_leader_id.as_ref()
                                                && leader != &node.me.id
                                                && transport.has_peer(leader)
                                            {
                                                transport.send_message(
                                                    leader,
                                                    TAG_JOIN_REQUEST,
                                                    frame[1..].to_vec(),
                                                );
                                            }
                                            return Some(0);
                                        }
                                        // A leader accepts either the joiner's own frame or a
                                        // relay from an already committed member. In both cases it
                                        // must have independently discovered the joiner's route;
                                        // an arbitrary L2 sender cannot nominate a backend.
                                        let trusted_relay = node
                                            .cluster_configuration
                                            .contains(&source_peer)
                                            && transport.has_peer(joiner);
                                        if joiner.is_empty() || (!direct && !trusted_relay) {
                                            return Some(0);
                                        }
                                        response_mac = transport
                                            .mac_for_peer(joiner)
                                            .unwrap_or(source_mac);
                                        Some(
                                            node.submit_join(
                                                Peer::voter(joiner.to_string(), service_name),
                                                node.millis(),
                                            )
                                            .unwrap_or(0),
                                        )
                                    })
                                    .unwrap_or(0);
                                catten_rt::logln!(
                                    "[dns] JOIN RECEIVED source={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} index={} term={} leader={}",
                                    source_mac[0],
                                    source_mac[1],
                                    source_mac[2],
                                    source_mac[3],
                                    source_mac[4],
                                    source_mac[5],
                                    accepted,
                                    node.current_term,
                                    node.state == NodeState::Leader
                                );
                                transport.send_response(
                                    response_mac,
                                    TAG_JOIN_REPLY,
                                    encode_join_reply(accepted),
                                );
                            }
                            Some(TAG_JOIN_REPLY) => {
                                if let Some(index) = decode_join_reply(&frame[1..]) {
                                    catten_rt::logln!(
                                        "[dns] JOIN ACK index={} term={} committed_members={}",
                                        index,
                                        node.current_term,
                                        node.cluster_configuration.all_members().len()
                                    );
                                    join_request_pending = false;
                                    // An index acknowledges only that the
                                    // leader appended (or deduplicated) JOIN;
                                    // it is not proof of committed admission.
                                    // Keep retrying idempotently until the
                                    // replicated configuration contains us.
                                    join_retry_at_ms =
                                        node.millis().saturating_add(JOIN_RETRY_MS);
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
                        term: node.current_term,
                        log_index,
                        reply: 0,
                        name,
                        connection: 0,
                        existing_local_generation: 0,
                    });
                }
            }
        }

        // The leader continuously turns signed placement policy plus the
        // committed, non-draining voter set into concrete desired replicas.
        // Generation fencing makes a leadership change or repeated pass
        // harmless.
        let placement_capacity =
            fresh_capacity_view(&catalog, &capacity_last_seen_ms, node.millis());
        reconcile_replica_placements(
            &mut node,
            &catalog,
            &mut pending_registers,
            &placement_capacity,
            &mut placement_gates,
            &mut placement_reassignments,
            &mut forced_reassignments,
        );

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
            let _attachments = catten_services::RequestAttachments::memory_only(message.memory);
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
            // Only the register opcodes may retain the attached connection for
            // a deferred reply; every other opcode must release it here.
            if message.connection != 0
                && !matches!(
                    message.opcode,
                    dns::OP_REGISTER | dns::OP_REGISTER_NAMED | dns::OP_REGISTER_DEPLOYMENT_NAMED
                )
            {
                ipc_close(message.connection);
            }
            match message.opcode {
                dns::OP_REGISTER => {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
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
                    if let Some(name) = read_named_bytes(&message) {
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
                    } else if message.connection != 0 {
                        ipc_close(message.connection);
                    }
                }

                dns::OP_REGISTER_DEPLOYMENT_NAMED => {
                    if let Some((name, deployment_generation)) =
                        read_deployment_registration(&message)
                    {
                        if let Some(result) = register_name(
                            &mut node,
                            ns_conn,
                            &transport,
                            &mut pending_registers,
                            &node_name,
                            &message,
                            name,
                            deployment_generation,
                        ) && message.reply != 0
                        {
                            ipc_reply(message.reply, result);
                        }
                    } else if message.connection != 0 {
                        ipc_close(message.connection);
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
                                        term: node.current_term,
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
                                        term: node.current_term,
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
                        if event_waiters.waiter_count() >= catten_services::broker::MAX_WAITERS
                            && catalog.lookup(&name).is_none()
                        {
                            if message.reply != 0 {
                                ipc_reply(message.reply, dns::ERR_BUSY);
                            }
                            continue;
                        }
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
                        let matches_active_owner = catalog
                            .lookup_owner(&name, &node_name)
                            .is_some_and(|entry| entry.generation == expected_generation);
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
                                    term: node.current_term,
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
                                            term: node.current_term,
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
                            match submit_deferred(
                                &mut node,
                                &mut pending_registers,
                                message.reply,
                                |_node| {
                                    Ok(encode_deploy(
                                        &request.name,
                                        request.object_id,
                                        if request.node_key == 0 {
                                            node_identity::key_from_name(&node_name).unwrap_or(0)
                                        } else {
                                            request.node_key
                                        },
                                        &request.digest,
                                        &request.descriptor,
                                    ))
                                },
                            ) {
                                Ok(()) => continue,
                                Err(code) => code,
                            }
                        }
                        Some(request) => match relay_to_leader(
                            &node,
                            &transport,
                            &mut pending_queries,
                            &mut next_query_id,
                            dns_session,
                            message.reply,
                            REMOTE_CALL_TIMEOUT_MS,
                            catten_services::rdeploy::TAG_REQUEST,
                            |session, request_id| {
                                catten_services::rdeploy::encode_request(
                                    &catten_services::rdeploy::Request {
                                        session,
                                        request_id,
                                        caller: node_name.clone(),
                                        artifact: request.name,
                                        object_id: request.object_id,
                                        node_key: request.node_key,
                                        digest: request.digest,
                                        descriptor: request.descriptor,
                                    },
                                )
                            },
                            |reply| PendingQueryKind::Deploy {
                                reply,
                            },
                        ) {
                            Some(code) => code,
                            None => continue,
                        },
                        None => dns::ERR_TOO_LARGE,
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_DEPLOY_RELEASE => {
                    let envelope =
                        read_moved_bytes(&message, charlotte_launch::release::MAX_RELEASE_LEN);
                    let result = match envelope {
                        Some(envelope) if node.state == NodeState::Leader => {
                            match submit_deferred(
                                &mut node,
                                &mut pending_registers,
                                message.reply,
                                |node| {
                                    let automatic_node =
                                        node_identity::key_from_name(&node_name).unwrap_or(0);
                                    let eligible_nodes = placement_nodes(node, &catalog);
                                    let capacity = release_capacity_view(
                                        &catalog,
                                        fresh_capacity_view(
                                            &catalog,
                                            &capacity_last_seen_ms,
                                            node.millis(),
                                        ),
                                        &envelope,
                                    );
                                    release_command(
                                        &envelope,
                                        &eligible_nodes,
                                        automatic_node,
                                        &capacity,
                                    )
                                },
                            ) {
                                Ok(()) => continue,
                                Err(code) => code,
                            }
                        }
                        Some(envelope) => match relay_to_leader(
                            &node,
                            &transport,
                            &mut pending_queries,
                            &mut next_query_id,
                            dns_session,
                            message.reply,
                            REMOTE_CALL_TIMEOUT_MS,
                            catten_services::rrelease::TAG_REQUEST,
                            |session, request_id| {
                                catten_services::rrelease::encode_request(
                                    &catten_services::rrelease::Request {
                                        session,
                                        request_id,
                                        caller: node_name.clone(),
                                        envelope,
                                    },
                                )
                            },
                            |reply| PendingQueryKind::Release {
                                reply,
                            },
                        ) {
                            Some(code) => code,
                            None => continue,
                        },
                        None => dns::ERR_TOO_LARGE,
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_DEPLOY_OPERATIONS => {
                    let bundle = read_moved_bytes(
                        &message,
                        charlotte_launch::operations_bundle::MAX_BUNDLE_LEN,
                    );
                    let result = match bundle {
                        Some(bundle) if node.state == NodeState::Leader => {
                            match submit_deferred(
                                &mut node,
                                &mut pending_registers,
                                message.reply,
                                |node| {
                                    let automatic_node =
                                        node_identity::key_from_name(&node_name).unwrap_or(0);
                                    let eligible_nodes = placement_nodes(node, &catalog);
                                    let capacity = operations_capacity_view(
                                        &catalog,
                                        fresh_capacity_view(
                                            &catalog,
                                            &capacity_last_seen_ms,
                                            node.millis(),
                                        ),
                                        &bundle,
                                    );
                                    operations_command(
                                        &bundle,
                                        &admission_trust,
                                        time_conn.as_ref(),
                                        &eligible_nodes,
                                        automatic_node,
                                        &capacity,
                                    )
                                },
                            ) {
                                Ok(()) => continue,
                                Err(code) => code,
                            }
                        }
                        Some(bundle) => match relay_to_leader(
                            &node,
                            &transport,
                            &mut pending_queries,
                            &mut next_query_id,
                            dns_session,
                            message.reply,
                            REMOTE_OPERATIONS_TIMEOUT_MS,
                            catten_services::roperations::TAG_REQUEST,
                            |session, request_id| {
                                catten_services::roperations::encode_request(
                                    &catten_services::roperations::Request {
                                        session,
                                        request_id,
                                        caller: node_name.clone(),
                                        bundle,
                                    },
                                )
                            },
                            |reply| PendingQueryKind::Operations {
                                reply,
                            },
                        ) {
                            Some(code) => code,
                            None => continue,
                        },
                        None => dns::ERR_TOO_LARGE,
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_SHUTDOWN_SUBMIT => {
                    let envelope =
                        read_moved_bytes(&message, charlotte_launch::shutdown::ENCODED_LEN);
                    let result = match envelope {
                        Some(envelope) if node.state == NodeState::Leader => {
                            match submit_deferred(
                                &mut node,
                                &mut pending_registers,
                                message.reply,
                                |_node| {
                                    shutdown_command(
                                        &envelope,
                                        &catalog,
                                        &admission_trust,
                                        time_conn.as_ref(),
                                    )
                                },
                            ) {
                                Ok(()) => continue,
                                Err(code) => code,
                            }
                        }
                        Some(envelope) => match relay_to_leader(
                            &node,
                            &transport,
                            &mut pending_queries,
                            &mut next_query_id,
                            dns_session,
                            message.reply,
                            REMOTE_CALL_TIMEOUT_MS,
                            catten_services::rshutdown::TAG_REQUEST,
                            |session, request_id| {
                                catten_services::rshutdown::encode_request(
                                    &catten_services::rshutdown::Request {
                                        session,
                                        request_id,
                                        caller: node_name.clone(),
                                        envelope,
                                    },
                                )
                            },
                            |reply| PendingQueryKind::Shutdown {
                                reply,
                            },
                        ) {
                            Some(code) => code,
                            None => continue,
                        },
                        None => dns::ERR_TOO_LARGE,
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_SHUTDOWN_QUERY => {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
                    if let Some(entry) = catalog.shutdown_intent(message.arg0) {
                        let mut bytes =
                            Vec::with_capacity(8 + charlotte_launch::shutdown::ENCODED_LEN);
                        bytes.extend_from_slice(&entry.generation.to_le_bytes());
                        bytes.extend_from_slice(&entry.envelope);
                        reply_move_bytes(message.reply, &bytes);
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_NOT_FOUND);
                    }
                    continue;
                }

                dns::OP_INGRESS_POLICY_SUBMIT => {
                    let envelope = read_moved_bytes(
                        &message,
                        charlotte_launch::ingress_policy::MAX_ENCODED_LEN,
                    );
                    let result = match envelope {
                        Some(envelope) if node.state == NodeState::Leader => {
                            match submit_deferred(
                                &mut node,
                                &mut pending_registers,
                                message.reply,
                                |_node| {
                                    ingress_policy_command(
                                        &envelope,
                                        &admission_trust,
                                        time_conn.as_ref(),
                                    )
                                },
                            ) {
                                Ok(()) => continue,
                                Err(code) => code,
                            }
                        }
                        Some(envelope) => match relay_to_leader(
                            &node,
                            &transport,
                            &mut pending_queries,
                            &mut next_query_id,
                            dns_session,
                            message.reply,
                            REMOTE_CALL_TIMEOUT_MS,
                            catten_services::ringress_policy::TAG_REQUEST,
                            |session, request_id| {
                                catten_services::ringress_policy::encode_request(
                                    &catten_services::ringress_policy::Request {
                                        session,
                                        request_id,
                                        caller: node_name.clone(),
                                        envelope,
                                    },
                                )
                            },
                            |reply| PendingQueryKind::IngressPolicy {
                                reply,
                            },
                        ) {
                            Some(code) => code,
                            None => continue,
                        },
                        None => dns::ERR_TOO_LARGE,
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_INGRESS_POLICY_QUERY => {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
                    if let Some(entry) = catalog.ingress_policy() {
                        let mut bytes = Vec::with_capacity(8 + entry.envelope.len());
                        bytes.extend_from_slice(&entry.generation.to_le_bytes());
                        bytes.extend_from_slice(&entry.envelope);
                        reply_move_bytes(message.reply, &bytes);
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_NOT_FOUND);
                    }
                }

                dns::OP_DEPLOY_QUERY => {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
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
                        if let Some(bytes) = encode_deployment_result(&entry) {
                            reply_move_bytes(message.reply, &bytes);
                        } else if message.reply != 0 {
                            ipc_reply(message.reply, dns::ERR_TOO_LARGE);
                        }
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
                        if let Some(bytes) = encode_deployment_result(&entry) {
                            reply_move_bytes(message.reply, &bytes);
                        } else if message.reply != 0 {
                            ipc_reply(message.reply, dns::ERR_TOO_LARGE);
                        }
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_NOT_FOUND);
                    }
                    continue;
                }

                dns::OP_DEPLOY_LIST => {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
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

                dns::OP_OPERATIONAL_LIST => {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
                    let stored = catalog.operational_bindings();
                    let bindings = stored
                        .iter()
                        .map(|(profile_name, entry)| {
                            charlotte_launch::operations_pickup::CatalogBinding {
                                generation: entry.generation,
                                bundle_sequence: entry.bundle_sequence,
                                sequence: entry.sequence,
                                expires_unix_seconds: entry.expires_unix_seconds,
                                profile_kind: entry.profile_kind,
                                release_name: &entry.release_name,
                                profile_name,
                                target_artifact: &entry.target_artifact,
                                object_key: &entry.object_key,
                                release_digest: entry.release_digest,
                                bundle_digest: entry.bundle_digest,
                                envelope_digest: entry.envelope_digest,
                                recipient_key_id: entry.recipient_key_id,
                                signing_key_id: entry.signing_key_id,
                                authorization_signature: entry.authorization_signature,
                            }
                        })
                        .collect::<Vec<_>>();
                    if let Some(len) =
                        charlotte_launch::operations_pickup::catalog_list_encoded_len(&bindings)
                    {
                        let mut bytes = vec![0; len];
                        if charlotte_launch::operations_pickup::encode_catalog_list(
                            &bindings, &mut bytes,
                        )
                        .is_some()
                        {
                            reply_move_bytes(message.reply, &bytes);
                        } else if message.reply != 0 {
                            ipc_reply(message.reply, dns::ERR_TOO_LARGE);
                        }
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_TOO_LARGE);
                    }
                    continue;
                }

                dns::OP_RELEASE_QUERY_NAMED => {
                    let release_name = read_named_bytes(&message);
                    if let Some(release_name) = release_name
                        && let Some(entry) = catalog.release(&release_name)
                    {
                        reply_move_bytes(message.reply, &entry.envelope);
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_NOT_FOUND);
                    }
                    continue;
                }

                dns::OP_DEPLOY_ROLLOUT_NAMED => {
                    let artifact = read_named_bytes(&message);
                    if let Some(artifact) = artifact
                        && let Some(deployment) = catalog.deployment(&artifact)
                    {
                        let placement = catalog.ingress_placement(&artifact);
                        let service_generation =
                            placement.as_ref().map_or(0, |view| view.service_generation);
                        let ready = placement.as_ref().map_or(0, |view| view.ready_nodes.len());
                        let state = if ready == deployment.replica_nodes.len() && ready != 0 {
                            clusterctl::ROLLOUT_READY
                        } else if ready == 0 {
                            clusterctl::ROLLOUT_COMMITTED
                        } else {
                            clusterctl::ROLLOUT_REPLACING
                        };
                        let status = clusterctl::RolloutStatus {
                            state,
                            deployment_generation: deployment.generation,
                            service_generation,
                            node_key: deployment.node_key,
                            desired_replicas: deployment
                                .replica_nodes
                                .len()
                                .try_into()
                                .unwrap_or(u16::MAX),
                            ready_replicas: ready.try_into().unwrap_or(u16::MAX),
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
                                            term: node.current_term,
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
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
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
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
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

                dns::OP_RAFT_FRAME => {
                    let frame_len = usize::try_from(message.arg0).unwrap_or(0);
                    let mut result = dns::ERR_BAD_OPCODE;
                    if message.memory != 0 && (15..=4096).contains(&frame_len) {
                        let (map_status, vaddr) = memory_map_any(message.memory, false);
                        if map_status == 0 {
                            let frame = unsafe {
                                core::slice::from_raw_parts(vaddr as *const u8, frame_len)
                            };
                            if frame[0..6] == local_mac
                                && u16::from_be_bytes([frame[12], frame[13]])
                                    == catten_services::raft::ETHERTYPE
                                && let Ok((tag, payload)) =
                                    catten_graft::wire::parse_tagged_payload(&frame[14..])
                                && (TAG_VOTE_REQUEST..=TAG_SNAPSHOT_RESPONSE).contains(&tag)
                            {
                                let source_mac: [u8; 6] = frame[6..12].try_into().unwrap_or([0; 6]);
                                if let Some(inbound) =
                                    transport.decode_inbound_parts(&source_mac, tag, payload)
                                {
                                    let millis = node.millis();
                                    drive_inbound(
                                        &mut node, &transport, source_mac, inbound, millis,
                                    );
                                }
                                result = 0;
                            }
                            memory_unmap(message.memory);
                        }
                        memory_close(message.memory);
                    } else if message.memory != 0 {
                        memory_close(message.memory);
                    }
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }

                dns::OP_INGRESS_MEMBERSHIP => {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
                    let service = charlotte_launch::ingress::ServiceId::unpack(message.arg0)
                        .and_then(|service| {
                            effective_ingress_bindings(&catalog, &ingress_services)
                                .into_iter()
                                .find(|binding| binding.service == service)
                        });
                    let snapshot = service
                        .filter(|_| {
                            node.can_serve_bounded_read(frouter::SNAPSHOT_SOURCE_MAX_AGE_MS)
                        })
                        .and_then(|service| {
                            ingress_membership_snapshot(
                                &node,
                                &transport,
                                &catalog,
                                local_mac,
                                service.backend_name.as_deref(),
                            )
                        });
                    match snapshot {
                        Some(snapshot) => reply_move_bytes(message.reply, &snapshot.encode()),
                        None if message.reply != 0 => {
                            // A partial discovery overlay must never silently
                            // become a different backend set on each node.
                            ipc_reply(message.reply, dns::ERR_BUSY);
                        }
                        None => {}
                    }
                }

                dns::OP_INGRESS_ASSIGNMENTS_NAMED => {
                    let name = read_moved_bytes(
                        &message,
                        charlotte_launch::deployment::MAX_ARTIFACT_NAME_LEN,
                    );
                    let Some(name) =
                        name.filter(|name| charlotte_launch::deployment::valid_artifact_name(name))
                    else {
                        if message.reply != 0 {
                            ipc_reply(message.reply, dns::ERR_TOO_LARGE);
                        }
                        continue;
                    };
                    let matches = effective_ingress_bindings(&catalog, &ingress_services)
                        .iter()
                        .filter(|binding| binding.backend_name.as_deref() == Some(name.as_slice()))
                        .cloned()
                        .collect::<Vec<_>>();
                    if matches.is_empty() {
                        if message.reply != 0 {
                            ipc_reply(message.reply, dns::ERR_NOT_FOUND);
                        }
                        continue;
                    }
                    let Some(bytes) = encode_effective_ingress_bindings(&matches) else {
                        if message.reply != 0 {
                            ipc_reply(message.reply, dns::ERR_TOO_LARGE);
                        }
                        continue;
                    };
                    reply_move_bytes(message.reply, &bytes);
                }

                dns::OP_INGRESS_ASSIGNMENTS => {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
                    let effective = effective_ingress_bindings(&catalog, &ingress_services);
                    if let Some(bytes) = encode_effective_ingress_bindings(&effective) {
                        reply_move_bytes(message.reply, &bytes);
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, dns::ERR_TOO_LARGE);
                    }
                }

                dns::OP_CLUSTER_SNAPSHOT => {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
                    let snapshot = cluster_observability_snapshot(
                        &node,
                        &transport,
                        &catalog,
                        local_mac,
                        &ingress_services,
                        &capacity_last_seen_ms,
                        cluster_observe::ControllerCounters {
                            capacity_reports_accepted,
                            capacity_commands_proposed,
                            placement_reassignments,
                            forced_reassignments,
                        },
                    );
                    match snapshot {
                        Some(bytes) => reply_move_bytes(message.reply, &bytes),
                        None if message.reply != 0 => {
                            // A keyhole must not make stale local state look
                            // like a current cluster-wide observation.
                            ipc_reply(message.reply, dns::ERR_BUSY);
                        }
                        None => {}
                    }
                }

                dns::OP_STATUS => {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
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
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
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
                        if ipc_reply_move(message.reply, cap, length as i64) != 0 {
                            memory_close(cap);
                        }
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
            // A pending entry is only valid while this node remains in the
            // term that submitted its command. After a term change the same
            // log index can hold a different command, so fail the stale entry
            // instead of completing it against the wrong result. Remote
            // registers wait on a remote reply rather than a local index and
            // are bounded by the queue cap below.
            if !matches!(&pending_registers[index], PendingRegistration::RemoteRegister { .. })
                && pending_registers[index].term() != node.current_term
            {
                let entry = pending_registers.swap_remove(index);
                abort_pending_entry(entry, &transport);
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
                | PendingRegistration::Placement {
                    log_index,
                    ..
                }
                | PendingRegistration::RemoteDeploy {
                    log_index,
                    ..
                }
                | PendingRegistration::RemoteRelease {
                    log_index,
                    ..
                }
                | PendingRegistration::RemoteOperations {
                    log_index,
                    ..
                }
                | PendingRegistration::RemoteShutdown {
                    log_index,
                    ..
                }
                | PendingRegistration::RemoteIngressPolicy {
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
                PendingRegistration::Placement {
                    artifact,
                    ..
                } => {
                    let generation = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(u64::from_le_bytes)
                        .unwrap_or(0);
                    if generation != 0 {
                        catten_rt::logln!(
                            "[dns] reconciled replica placement {:?} generation={}",
                            core::str::from_utf8(&artifact).unwrap_or("<invalid>"),
                            generation
                        );
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
                PendingRegistration::RemoteRelease {
                    peer,
                    session,
                    request_id,
                    ..
                } => {
                    let result = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(i64::from_le_bytes)
                        .unwrap_or(dns::ERR_NOT_FOUND);
                    transport.send_message(
                        &peer,
                        catten_services::rrelease::TAG_REPLY,
                        catten_services::rrelease::encode_reply(session, request_id, result),
                    );
                }
                PendingRegistration::RemoteOperations {
                    peer,
                    session,
                    request_id,
                    ..
                } => {
                    let result = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(i64::from_le_bytes)
                        .unwrap_or(dns::ERR_NOT_FOUND);
                    transport.send_message(
                        &peer,
                        catten_services::roperations::TAG_REPLY,
                        catten_services::roperations::encode_reply(session, request_id, result),
                    );
                }
                PendingRegistration::RemoteShutdown {
                    peer,
                    session,
                    request_id,
                    ..
                } => {
                    let result = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(i64::from_le_bytes)
                        .unwrap_or(dns::ERR_NOT_FOUND);
                    transport.send_message(
                        &peer,
                        catten_services::rshutdown::TAG_REPLY,
                        catten_services::rshutdown::encode_reply(session, request_id, result),
                    );
                }
                PendingRegistration::RemoteIngressPolicy {
                    peer,
                    session,
                    request_id,
                    ..
                } => {
                    let result = node
                        .command_result(log_index)
                        .and_then(|bytes| bytes.get(..8))
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                        .map(i64::from_le_bytes)
                        .unwrap_or(dns::ERR_NOT_FOUND);
                    transport.send_message(
                        &peer,
                        catten_services::ringress_policy::TAG_REPLY,
                        catten_services::ringress_policy::encode_reply(session, request_id, result),
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
                            term: node.current_term,
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
                            let (local_generation, _) = unsafe { wait_reply(local_reg) };
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
                                term: node.current_term,
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

        if pending_registers.len() > MAX_PENDING_REGISTRATIONS {
            let excess = pending_registers.len() - MAX_PENDING_REGISTRATIONS;
            for _ in 0..excess {
                let entry = pending_registers.remove(0);
                abort_pending_entry(entry, &transport);
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

            let still_active = catalog
                .lookup_owner(&publication.name, &node_name)
                .is_some_and(|entry| entry.generation == publication.generation);
            // A publication becomes stale only when the catalog has moved past
            // its generation (a replacement or migration) or when this dns
            // itself tore the endpoint down. An absent entry is NOT stale by
            // itself: for a follower the activate may simply still be
            // replicating when the leader's register reply arrives, and
            // cleaning up there would unregister a live local service.
            let superseded = catalog
                .lookup_owner(&publication.name, &node_name)
                .is_some_and(|entry| entry.generation != publication.generation);
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
                            term: node.current_term,
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
            &mut next_reply_ordinal,
            &mut remote_calls_served,
            &transport,
            node.millis(),
        );
        expire_remote_calls(&mut in_flight_calls, node.millis());
        expire_queries(&mut pending_queries, node.millis());
        // The transport's reply-retry guard compares against this clock; keep
        // it fresh every tick so a lost AppendEntries response cannot freeze
        // retransmission.
        transport.set_current_millis(node.millis());
        advance_raft_clock(
            &mut node,
            tick_due,
            heartbeat_interval_ms,
            &mut last_heartbeat_broadcast,
            &mut timer_armed,
        );
        if logged_membership_epoch != node.membership_epoch() {
            logged_membership_epoch = node.membership_epoch();
            catten_rt::logln!(
                "[dns] MEMBERSHIP epoch={} joint={} members={} commit={} last={} \
                 finalize_pending={} term={} leader={}",
                logged_membership_epoch,
                node.cluster_configuration.is_joint_consensus(),
                node.cluster_configuration.all_members().len(),
                node.commit_index,
                node.log_store.last_index(),
                node.finalize_configuration_pending,
                node.current_term,
                node.state == NodeState::Leader
            );
        }
        if logged_raft_term != node.current_term || logged_raft_state != node.state {
            // Filter and freshness state is leader-local authority. Rebuild it
            // after every leadership/term transition instead of allowing an
            // observation accepted under an earlier control epoch to extend
            // the new leader's lease.
            capacity_filters.clear();
            capacity_last_seen_ms.clear();
            placement_gates.clear();
            logged_raft_term = node.current_term;
            logged_raft_state = node.state;
            catten_rt::logln!(
                "[dns] RAFT STATE state={:?} term={} leader={:?} millis={} pending={} queued={}",
                node.state,
                node.current_term,
                node.known_leader_id,
                node.millis(),
                transport.pending_send_count(),
                transport.outbound_count()
            );
        }
        if node.cluster_configuration.is_joint_consensus()
            && node.millis() >= next_joint_diagnostic_ms
        {
            next_joint_diagnostic_ms = node.millis().saturating_add(5_000);
            catten_rt::logln!(
                "[dns] JOINT PROGRESS epoch={} commit={} last={} pending={} queued={} \
                 append_tx={} append_rx={}",
                node.membership_epoch(),
                node.commit_index,
                node.log_store.last_index(),
                transport.pending_send_count(),
                transport.outbound_count(),
                transport.acknowledged_count(TAG_APPEND_REQUEST),
                transport.acknowledged_count(TAG_APPEND_RESPONSE)
            );
        }
        publish_status(&node, &catalog);
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}

catten_rt::entry!(main);
