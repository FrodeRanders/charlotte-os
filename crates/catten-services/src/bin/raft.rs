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
    ns,
    raft,
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
    ipc_reply_wait,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map,
    memory_unmap,
    thread_exit,
};

const LOOP_TICK_MS: u64 = 25;

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
    let value =
        unsafe { core::slice::from_raw_parts(SCRATCH_VADDR as *const u8, length).to_vec() };
    memory_unmap(cap);
    memory_close(cap);
    Some(value)
}

fn reply_payload(reply: u64, payload: Result<Vec<u8>, catten_graft::wire::WireError>) {
    if let Ok(payload) = payload {
        if let Some(memory) = write_payload_to_mem(&payload) {
            ipc_reply_move(reply, memory, payload.len() as i64);
            return;
        }
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
    let node_id = match ctx.manifest_value(NODE_ID_KEY) {
        Some(ManifestValue::Bytes(bytes)) => core::str::from_utf8(bytes).unwrap_or("r1"),
        _ => "r1",
    };
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

    let endpoint = ipc_endpoint_create(raft::INTERFACE, raft::VERSION, 8);
    if endpoint == 0 {
        fatal(2);
    }
    if ipc_endpoint_bind_cq(endpoint, 0) != 0 {
        fatal(5);
    }
    config::write::<u32>(0, 3);

    let name_u64 = catten_services::name(alloc::format!("raft-{}", node_id).as_bytes());

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
    let transport = Arc::new(CharlotteTransport::new());

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

        let peer_name = catten_services::name(alloc::format!("raft-{}", peer_id).as_bytes());
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

    loop {
        // Endpoint readiness and transport completions wake this reactor
        // immediately. The bounded wait itself supplies Raft's periodic clock,
        // avoiding a separate detached-timer completion and wake path.
        let (_, timed_out) = cq_wait_timeout(1, LOOP_TICK_MS, 0);

        while unsafe { cq_read(cq.base, cq.entries) }.is_some() {}

        let completed = node.poll_transport(node.millis());
        if completed > 0 {
            config::write::<u32>(16, completed as u32);
        }

        // Keep one deferred name-service lookup outstanding for each missing
        // peer. Registration completes that call; the reactor only polls the
        // existing call and never creates retry storms or blocks on a peer.
        // Membership is replicated, so discovery must follow the committed
        // configuration rather than remaining frozen at the boot manifest.
        let active_peer_ids = node
            .cluster_configuration
            .all_members()
            .into_iter()
            .map(|peer| peer.id.clone())
            .collect::<Vec<_>>();
        peer_specs.retain(|(peer_id, _, pending)| {
            if active_peer_ids.contains(peer_id) {
                true
            } else {
                if *pending != 0 {
                    ipc_close(*pending);
                }
                transport.remove_peer(peer_id);
                false
            }
        });
        for peer in node.cluster_configuration.all_members() {
            if peer.id != node.me.id && !peer_specs.iter().any(|spec| spec.0 == peer.id) {
                let peer_name = if peer.service_name != 0 {
                    peer.service_name
                } else {
                    catten_services::name(alloc::format!("raft-{}", peer.id).as_bytes())
                };
                peer_specs.push((peer.id.clone(), peer_name, 0));
            }
        }
        for (peer_id, peer_name, pending) in &mut peer_specs {
            poll_peer_discovery(ns_conn, peer_id, *peer_name, pending, &transport);
        }

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

                _ => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }
            }
        }

        if timed_out != 0 {
            node.set_millis(node.millis() + LOOP_TICK_MS);
            if node.check_timeout() {
                node.start_election(node.millis());
            }
        }

        if node.state == NodeState::Leader {
            node.broadcast_heartbeat(node.millis());
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
