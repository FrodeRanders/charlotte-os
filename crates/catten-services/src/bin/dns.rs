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
    collections::BTreeMap,
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
        NameCatalog,
        encode_register,
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
    cq_read,
    cq_wait_timeout,
    ipc_close,
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
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map,
    memory_unmap,
    thread_exit,
};
use charlotte_protocol_msg::unpack_address_and_len;

const LOOP_TICK_MS: u64 = 25;
const REPLY_SPINS: u64 = u64::MAX;
const RX_SCRATCH: usize = 0x0000_0000_0090_0000;
const LIST_SCRATCH: usize = 0x0000_0000_0090_1000;
const CATALOG_SCRATCH: usize = 0x0000_0000_0090_2000;

const CLUSTER_KEY: u64 = manifest_key(b"cluster");
const EXPECTED_PEERS_KEY: u64 = manifest_key(b"peers");
const MEMBER_KEY: u64 = manifest_key(b"member");
const ELECTION_KEY: u64 = manifest_key(b"elect-ms");

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
    // Deferred registers awaiting commit replication: (log index, reply token,
    // name, service connection).
    let mut pending_registers: Vec<(u64, u64, Vec<u8>, u64)> = Vec::new();
    // In-flight remote calls awaiting a reply: (call id, client reply token).
    let mut in_flight_calls: Vec<(u64, u64)> = Vec::new();
    let mut next_call_id: u64 = 1;

    loop {
        let (_, timed_out) = cq_wait_timeout(1, LOOP_TICK_MS, 0);
        while unsafe { cq_read(cq.base, cq.entries) }.is_some() {}

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
                                if let Some((call_id, target, opcode, arg)) =
                                    catten_services::rcall::decode_request(frame)
                                {
                                    let result = match catalog.lookup(&target) {
                                        Some(owner) if owner == node_name => {
                                            invoke_local(ns_conn, &target, opcode, arg)
                                        }
                                        _ => dns::ERR_NOT_FOUND,
                                    };
                                    remote_calls_served = remote_calls_served.wrapping_add(1);
                                    config::write::<u32>(36, remote_calls_served);
                                    let reply =
                                        catten_services::rcall::encode_reply(call_id, result);
                                    if let Some(peer) = transport.peer_id_for_mac(&source_mac) {
                                        transport.send_message(
                                            &peer,
                                            catten_services::rcall::TAG_REPLY,
                                            reply,
                                        );
                                    }
                                }
                            }
                            Some(catten_services::rcall::TAG_REPLY) => {
                                // Complete the matching in-flight OP_CALL.
                                if let Some((call_id, result)) =
                                    catten_services::rcall::decode_reply(frame)
                                    && let Some(index) =
                                        in_flight_calls.iter().position(|(id, _)| *id == call_id)
                                {
                                    let (_, reply) = in_flight_calls.swap_remove(index);
                                    if reply != 0 {
                                        ipc_reply(reply, result);
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
                                pending_registers.push((
                                    index,
                                    message.reply,
                                    name,
                                    message.connection,
                                ));
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
                    } else {
                        match catalog.lookup(&name) {
                            Some(owner) if owner == node_name => {
                                // Local: resolve through the node-local name
                                // service and hand back the connection. A
                                // catalog-only registration has no connection
                                // to delegate, so fall back to a scalar
                                // RESULT_LOCAL.
                                let lookup = ipc_scalar_call(
                                    ns_conn,
                                    ns::OP_TRY_LOOKUP,
                                    catten_services::name(&name),
                                );
                                let (generation, conn) = if lookup != 0 {
                                    unsafe { wait_reply(lookup, REPLY_SPINS) }
                                } else {
                                    (0, 0)
                                };
                                if generation >= 1 && conn != 0 && message.reply != 0 {
                                    ipc_reply_connection(
                                        message.reply,
                                        conn,
                                        IpcRights::SEND | IpcRights::CALL,
                                        dns::RESULT_LOCAL,
                                    );
                                    continue;
                                }
                                if message.reply != 0 {
                                    ipc_reply(message.reply, dns::RESULT_LOCAL);
                                }
                                continue;
                            }
                            Some(owner) => {
                                // Remote: reply RESULT_REMOTE and move a memory
                                // object carrying the hosting node's id.
                                if message.reply != 0 && !owner.is_empty() {
                                    let cap = memory_alloc(1);
                                    if cap != 0 && memory_map(cap, LIST_SCRATCH, true) == 0 {
                                        unsafe {
                                            core::ptr::copy_nonoverlapping(
                                                owner.as_ptr(),
                                                LIST_SCRATCH as *mut u8,
                                                owner.len(),
                                            );
                                        }
                                        memory_unmap(cap);
                                        ipc_reply_move(message.reply, cap, dns::RESULT_REMOTE);
                                        continue;
                                    }
                                    if cap != 0 {
                                        memory_close(cap);
                                    }
                                }
                                dns::ERR_NOT_FOUND
                            }
                            None => dns::ERR_NOT_FOUND,
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
                    // page: [count:u32][len:u8 name node_len:u8 node]*.
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
                    for (name, node) in entries.iter() {
                        let name_len = name.len().min(255);
                        let node_len = node.len().min(255);
                        if length + 2 + name_len + node_len > 4096 {
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
                                node.as_ptr(),
                                (CATALOG_SCRATCH + length + 2 + name_len) as *mut u8,
                                node_len,
                            );
                        }
                        length += 2 + name_len + node_len;
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
                    } else {
                        match catalog.lookup(&name) {
                            Some(owner) if owner == node_name => {
                                invoke_local(ns_conn, &name, opcode, arg)
                            }
                            Some(owner) => {
                                // Remote: relay to the hosting node's dns over
                                // the reliable message layer.
                                let owner_str =
                                    core::str::from_utf8(&owner).unwrap_or("").to_string();
                                if let Some(_mac) = transport.mac_for_peer(&owner_str) {
                                    let call_id = next_call_id;
                                    next_call_id = next_call_id.wrapping_add(1);
                                    in_flight_calls.push((call_id, message.reply));
                                    let frame = catten_services::rcall::encode_request(
                                        call_id, &name, opcode, arg,
                                    );
                                    transport.send_message(
                                        &owner_str,
                                        catten_services::rcall::TAG_REQUEST,
                                        frame,
                                    );
                                    continue; // reply completes when the remote REPLY arrives
                                }
                                dns::ERR_NOT_FOUND
                            }
                            None => dns::ERR_NOT_FOUND,
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
            let (log_index, _reply, _name, _connection) = &pending_registers[index];
            if !node.is_committed(*log_index) {
                index += 1;
                continue;
            }
            let (log_index, reply, name, connection) = pending_registers.swap_remove(index);
            // Register the service connection with the node-local name service
            // (catalog-only registrations carry no connection).
            let result = if connection != 0 {
                let local_reg = ipc_scalar_call_connection(
                    ns_conn,
                    ns::OP_REGISTER,
                    catten_services::name(&name),
                    connection,
                    IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
                );
                if local_reg != 0 {
                    let (generation, _) = unsafe { wait_reply(local_reg, REPLY_SPINS) };
                    if generation >= 1 {
                        0
                    } else {
                        dns::ERR_TOO_LARGE
                    }
                } else {
                    dns::ERR_TOO_LARGE
                }
            } else {
                0
            };
            if reply != 0 {
                ipc_reply(reply, result);
            }
            let _ = log_index;
        }

        // --- Raft clock ---
        if timed_out != 0 {
            node.set_millis(node.millis() + LOOP_TICK_MS);
            if node.check_timeout() {
                node.start_election(node.millis());
            }
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
