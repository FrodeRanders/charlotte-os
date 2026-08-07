//! Cluster discovery service — Ethernet broadcast bootstrap for cluster peers.
//!
//! Runs as a standalone EL0 service. Periodically broadcasts probe frames on
//! EtherType `0x88B6` and collects unicast responses from other nodes. Probe
//! frames carry the sender's identity so that **all nodes on the L2 segment
//! passively learn about each other** — a single probe from a new node is
//! sufficient for the whole cluster to learn its existence.
//!
//! Incoming frames are delivered by the frame demultiplexer ([`frouter`])
//! through this service's `OP_FRAME` ingress; outgoing probes and responses
//! are transmitted directly through the NIC driver. This service therefore
//! never owns `net OP_RECV`, letting relmsg and future consumers share the
//! single NIC receive path.
//!
//! IPC endpoint opcodes:
//! - OP_PROBE: force an immediate probe broadcast, reply with current peer count
//! - OP_LIST_PEERS: return cached peers immediately (non-blocking)
//! - OP_STATUS: return running + peer count
//! - OP_SHUTDOWN: exit
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    vec::Vec,
};

use catten_rt::{
    Context,
    ManifestValue,
    config,
};
use catten_services::{
    disco,
    net,
    ns,
    raft,
    sleep_ms,
    wait_for_local_ready,
    wait_reply,
};
use catten_syscall::{
    IpcRights,
    cq_wait_timeout,
    ipc_close,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_reply_poll,
    ipc_reply_poll_with_memory,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_scalar_call_move,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
    thread_exit,
};
use charlotte_protocol_disco::{
    BROADCAST_MAC,
    CLUSTER_ID_LEN,
    FLAG_PROBE,
    FLAG_RESPONSE,
    FRAME_HEADER_SIZE,
    PeerClusterInfo,
    ROLE_CANDIDATE,
    ROLE_FOLLOWER,
    ROLE_LEADER,
    ROLE_NO_CLUSTER,
    ROLE_UNKNOWN,
    build_cluster_answer,
    build_disco_frame,
    build_extended_payload,
    parse_disco_frame,
    parse_extended_payload,
};
use charlotte_protocol_net::decode_status;

const REPLY_SPINS: u64 = 50_000_000;

const RAPID_PROBE_COUNT: usize = 3;
const RAPID_PROBE_INTERVAL_MS: u64 = 200;
const BACKGROUND_PROBE_INTERVAL_MS: u64 = 15_000;
const RAFT_STATUS_REFRESH_MS: u64 = 2_000;

/// This node's cluster posture as learned from the local raft service: the
/// role maps directly to the discovery payload's `ROLE_*` values.
#[derive(Clone)]
struct ClusterInfo {
    role: u8,
    raft_id: Vec<u8>,
    leader_id: Vec<u8>,
}

impl Default for ClusterInfo {
    fn default() -> Self {
        Self {
            role: ROLE_NO_CLUSTER,
            raft_id: Vec::new(),
            leader_id: Vec::new(),
        }
    }
}

/// Non-blocking refresh of [`ClusterInfo`]: periodically looks up the local
/// raft service (`raft-{node identity}`) and polls its cluster status. Never
/// blocks frame processing: the whole exchange rides on reply polls.
struct ClusterProbe {
    info: ClusterInfo,
    pending_lookup: u64,
    pending_status: u64,
    status_conn: u64,
    next_refresh_ms: u64,
}

impl ClusterProbe {
    fn new() -> Self {
        Self {
            info: ClusterInfo::default(),
            pending_lookup: 0,
            pending_status: 0,
            status_conn: 0,
            next_refresh_ms: 0,
        }
    }

    fn maybe_start(&mut self, ns_conn: u64, identity: &[u8], tick_ms: u64) {
        if self.pending_lookup != 0 || self.pending_status != 0 || tick_ms < self.next_refresh_ms {
            return;
        }
        self.next_refresh_ms = tick_ms + RAFT_STATUS_REFRESH_MS;
        let raft_name = catten_services::raft_name(identity);
        self.pending_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, raft_name);
        catten_syscall::el0_log(0x4449_5343, raft_name | 0x0000_0000_0100_0000);
    }

    /// Advance the refresh state machine. Returns true when the advertised
    /// cluster info changed.
    fn poll(&mut self) -> bool {
        let mut changed = false;
        if self.pending_lookup != 0 {
            let (status, generation, conn) = ipc_reply_poll(self.pending_lookup);
            if status == 0 {
                ipc_close(self.pending_lookup);
                self.pending_lookup = 0;
                catten_syscall::el0_log(0x4449_5343, 0x2222 | ((generation.min(0xff) as u64) << 8) | ((conn != 0) as u64));
                if generation >= 1 && conn != 0 {
                    self.status_conn = conn;
                    self.pending_status = ipc_scalar_call(conn, raft::OP_CLUSTER_STATUS, 0);
                } else {
                    if conn != 0 {
                        ipc_close(conn);
                    }
                    changed |= self.set_not_in_cluster();
                }
            } else if status != 1 {
                ipc_close(self.pending_lookup);
                self.pending_lookup = 0;
                changed |= self.set_not_in_cluster();
            }
        }
        if self.pending_status != 0 {
            let (status, _result, _conn, memory) = ipc_reply_poll_with_memory(self.pending_status);
            if status == 0 {
                ipc_close(self.pending_status);
                self.pending_status = 0;
                if self.status_conn != 0 {
                    ipc_close(self.status_conn);
                    self.status_conn = 0;
                }
                catten_syscall::el0_log(0x4449_5343, 0x3333 | ((memory != 0) as u64));
                if memory != 0 {
                    changed |= self.consume_status(memory);
                } else {
                    changed |= self.set_not_in_cluster();
                }
            } else if status != 1 {
                ipc_close(self.pending_status);
                self.pending_status = 0;
                if self.status_conn != 0 {
                    ipc_close(self.status_conn);
                    self.status_conn = 0;
                }
                if memory != 0 {
                    memory_close(memory);
                }
                changed |= self.set_not_in_cluster();
            }
        }
        changed
    }

    fn consume_status(&mut self, memory: u64) -> bool {
        let mut changed = false;
        let (rstat_scratch_map_status, rstat_scratch_vaddr) = memory_map_any(memory, false);
        if rstat_scratch_map_status == 0 {
            let bytes = unsafe { core::slice::from_raw_parts(rstat_scratch_vaddr as *const u8, 4096) };
            if let Some((state, _term, _commit, _members, leader_id, self_id)) =
                raft::parse_cluster_status(bytes)
            {
                let role = match state {
                    1 => ROLE_FOLLOWER,
                    2 => ROLE_CANDIDATE,
                    3 => ROLE_LEADER,
                    _ => ROLE_NO_CLUSTER,
                };
                if self.info.role != role
                    || self.info.raft_id != self_id
                    || self.info.leader_id != leader_id
                {
                    self.info = ClusterInfo {
                        role,
                        raft_id: self_id.to_vec(),
                        leader_id: leader_id.to_vec(),
                    };
                    changed = true;
                    catten_syscall::el0_log(
                        0x4449_5343,
                        (role as u64)
                            | ((self_id.len().min(0xff) as u64) << 8)
                            | ((leader_id.len().min(0xff) as u64) << 16),
                    );
                }
            } else {
                changed |= self.set_not_in_cluster();
            }
            memory_unmap(memory);
        }
        memory_close(memory);
        changed
    }

    fn set_not_in_cluster(&mut self) -> bool {
        let changed = self.info.role != ROLE_NO_CLUSTER
            || !self.info.raft_id.is_empty()
            || !self.info.leader_id.is_empty();
        if changed {
            self.info = ClusterInfo::default();
        }
        changed
    }
}

#[allow(dead_code)]
struct DiscoveredPeer {
    mac: [u8; 6],
    node_id: Vec<u8>,
    service_name: u64,
    deadline_ms: u64,
    role: u8,
    raft_id: Vec<u8>,
    leader_id: Vec<u8>,
}

static mut DIAG_RX_RAW: u32 = 0;
static mut DIAG_SENT_OK: u32 = 0;
static mut DIAG_SENT_FAIL: u32 = 0;
static mut DIAG_DECODED: u32 = 0;
static mut DIAG_CALLED: u32 = 0;

fn publish_diag() {
    unsafe {
        config::write::<u32>(12, DIAG_RX_RAW);
        config::write::<u32>(16, DIAG_SENT_OK);
        config::write::<u32>(20, DIAG_SENT_FAIL);
        config::write::<u32>(24, DIAG_DECODED);
        config::write::<u32>(28, DIAG_CALLED);
    }
}

fn heartbeat(beat: u32) {
    config::write::<u32>(36, beat);
}

fn send_raw_frame(net_conn: u64, frame: &[u8]) -> bool {
    if frame.len() > 4096 {
        return false;
    }
    config::write::<u32>(40, 1);
    let cap = memory_alloc(1);
    if cap == 0 {
        return false;
    }
    config::write::<u32>(40, 2);
        let (tx_scratch_map_status, tx_scratch_vaddr) = memory_map_any(cap, true);
    if tx_scratch_map_status != 0 {
        memory_close(cap);
        return false;
    }
    config::write::<u32>(40, 3);
    unsafe {
        core::ptr::copy_nonoverlapping(frame.as_ptr(), tx_scratch_vaddr as *mut u8, frame.len());
    }
    memory_unmap(cap);
    let call = ipc_scalar_call_move(net_conn, net::OP_SEND, frame.len() as u64, cap);
    if call == 0 {
        memory_close(cap);
        unsafe { DIAG_SENT_FAIL = DIAG_SENT_FAIL.wrapping_add(1) };
        config::write::<u32>(40, 0xff);
        return false;
    }
    unsafe { DIAG_CALLED = DIAG_CALLED.wrapping_add(1) };
    config::write::<u32>(40, 4);
    let (result, _) = unsafe { wait_reply(call, REPLY_SPINS) };
    config::write::<u32>(40, 5);
    if result == 0 {
        unsafe { DIAG_SENT_OK = DIAG_SENT_OK.wrapping_add(1) };
        true
    } else {
        unsafe { DIAG_SENT_FAIL = DIAG_SENT_FAIL.wrapping_add(1) };
        false
    }
}

fn send_probe(
    net_conn: u64,
    local_mac: [u8; 6],
    cluster_id: &[u8; CLUSTER_ID_LEN],
    node_id: &[u8],
    service_name: u64,
    cluster: &ClusterInfo,
) {
    let mut payload_buf = [0u8; 256];
    let payload_len = build_extended_payload(
        &mut payload_buf,
        node_id,
        service_name,
        cluster.role,
        &cluster.raft_id,
        &cluster.leader_id,
    );
    if payload_len == 0 {
        return;
    }
    let mut frame = alloc::vec![0u8; FRAME_HEADER_SIZE + payload_len];
    let mut header = [0u8; FRAME_HEADER_SIZE];
    build_disco_frame(&mut header, BROADCAST_MAC, local_mac, FLAG_PROBE, cluster_id);
    frame[..FRAME_HEADER_SIZE].copy_from_slice(&header);
    frame[FRAME_HEADER_SIZE..].copy_from_slice(&payload_buf[..payload_len]);
    send_raw_frame(net_conn, &frame);
}

fn send_response(
    net_conn: u64,
    dst_mac: [u8; 6],
    local_mac: [u8; 6],
    cluster_id: &[u8; CLUSTER_ID_LEN],
    node_id: &[u8],
    service_name: u64,
    cluster: &ClusterInfo,
) {
    let mut payload_buf = [0u8; 256];
    let payload_len = build_extended_payload(
        &mut payload_buf,
        node_id,
        service_name,
        cluster.role,
        &cluster.raft_id,
        &cluster.leader_id,
    );
    if payload_len == 0 {
        return;
    }
    let mut response = alloc::vec![0u8; FRAME_HEADER_SIZE + payload_len];
    let mut header = [0u8; FRAME_HEADER_SIZE];
    build_disco_frame(&mut header, dst_mac, local_mac, FLAG_RESPONSE, cluster_id);
    response[..FRAME_HEADER_SIZE].copy_from_slice(&header);
    response[FRAME_HEADER_SIZE..].copy_from_slice(&payload_buf[..payload_len]);
    send_raw_frame(net_conn, &response);
}

fn learn_peer(
    peers: &mut BTreeMap<[u8; 6], DiscoveredPeer>,
    mac: [u8; 6],
    node_id: &[u8],
    service_name: u64,
    role: u8,
    raft_id: &[u8],
    leader_id: &[u8],
    tick_ms: u64,
    ttl_ms: u64,
) {
    peers.insert(
        mac,
        DiscoveredPeer {
            mac,
            node_id: node_id.to_vec(),
            service_name,
            deadline_ms: tick_ms + ttl_ms,
            role,
            raft_id: raft_id.to_vec(),
            leader_id: leader_id.to_vec(),
        },
    );
    config::write::<u32>(8, peers.len() as u32);
}

fn evict_expired(peers: &mut BTreeMap<[u8; 6], DiscoveredPeer>, tick_ms: u64) {
    let expired: Vec<[u8; 6]> =
        peers.iter().filter(|(_, p)| tick_ms > p.deadline_ms).map(|(m, _)| *m).collect();
    if !expired.is_empty() {
        for mac in expired {
            peers.remove(&mac);
        }
        config::write::<u32>(8, peers.len() as u32);
    }
}

/// Process one discovery frame delivered through `OP_FRAME` from the frouter.
#[allow(clippy::too_many_arguments)]
fn handle_frame(
    net_conn: u64,
    local_mac: [u8; 6],
    cluster_id: &[u8; CLUSTER_ID_LEN],
    node_id: &[u8],
    own_service_name: u64,
    cluster: &ClusterInfo,
    peers: &mut BTreeMap<[u8; 6], DiscoveredPeer>,
    tick_ms: u64,
    frame: &[u8],
) {
    if let Some((_version, flags, source_mac, frame_cluster_id)) = parse_disco_frame(frame) {
        unsafe { DIAG_DECODED = DIAG_DECODED.wrapping_add(1) };
        if frame_cluster_id == *cluster_id && source_mac != local_mac {
            let payload = &frame[FRAME_HEADER_SIZE..];
            if let Some((peer_id, service_name, cluster_block)) = parse_extended_payload(payload)
            {
                let (role, raft_id, leader_id) =
                    cluster_block.unwrap_or((ROLE_UNKNOWN, &[], &[]));
                learn_peer(
                    peers,
                    source_mac,
                    peer_id,
                    service_name,
                    role,
                    raft_id,
                    leader_id,
                    tick_ms,
                    disco::PEER_TTL_MS,
                );
            }
            if (flags & FLAG_PROBE) != 0 {
                send_response(
                    net_conn,
                    source_mac,
                    local_mac,
                    cluster_id,
                    node_id,
                    own_service_name,
                    cluster,
                );
            }
        }
    }
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(0, 1);
    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(0, 2);

    let lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, net::NAME);
    if lookup == 0 {
        unsafe { thread_exit() };
    }
    let (generation, net_conn) = unsafe { wait_reply(lookup, REPLY_SPINS) };
    if generation < 1 || net_conn == 0 {
        unsafe { thread_exit() };
    }

    let status_call = ipc_scalar_call(net_conn, net::OP_STATUS, 0);
    if status_call == 0 {
        unsafe { thread_exit() };
    }
    let (status, _) = unsafe { wait_reply(status_call, REPLY_SPINS) };
    let (link, local_mac) = decode_status(status);
    if link == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 3);

    let cluster_raw = ctx.manifest_value(charlotte_launch::manifest_key(b"cluster"));
    let mnemonic: Vec<u8> = match &cluster_raw {
        Some(ManifestValue::Bytes(raw)) if !raw.is_empty() => raw.to_vec(),
        _ => b"default".to_vec(),
    };
    let cluster_id_raw: [u8; CLUSTER_ID_LEN] = {
        let mut id = [0u8; CLUSTER_ID_LEN];
        let len = mnemonic.len().min(CLUSTER_ID_LEN);
        id[..len].copy_from_slice(&mnemonic[..len]);
        id
    };

    // The node's name is the persisted identity derived from its NIC MAC and
    // the cluster mnemonic, so every node on the segment carries the same
    // stable identity in its probes.
    let node_id = match catten_services::node_identity::NodeIdentity::load_or_create(
        ns_conn,
        &mnemonic,
        Some(local_mac),
    ) {
        Some(identity) => identity.name,
        None => {
            config::write::<u32>(0, 0xff03);
            unsafe { thread_exit() };
        }
    };
    config::write::<u32>(0, 4);

    let ep = ipc_endpoint_create(disco::INTERFACE, disco::VERSION, 8);
    if ep == 0 {
        unsafe { thread_exit() };
    }
    if ipc_endpoint_bind_cq(ep, 0) != 0 {
        unsafe { thread_exit() };
    }
    let registration = ipc_scalar_call_connection(
        ns_conn,
        ns::OP_REGISTER,
        disco::NAME,
        ep,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if registration == 0 {
        unsafe { thread_exit() };
    }
    let (reg_gen, _) = unsafe { wait_reply(registration, REPLY_SPINS) };
    if reg_gen < 1 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 5);
    publish_diag();

    let own_service_name = disco::NAME;
    let mut peers: BTreeMap<[u8; 6], DiscoveredPeer> = BTreeMap::new();
    let mut cluster = ClusterProbe::new();

    // Wait until this node has finished booting before broadcasting, so the
    // NIC and the two-node socket transport have settled. Probes sent during
    // the boot storm are silently lost and never retried.
    if !wait_for_local_ready(ns_conn) {
        config::write::<u32>(0, 0xff10);
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 6);

    // Send rapid bootstrap probes before entering the reactor. Subsequent
    // background probes are paced inside the loop so a blocked receive can
    // never starve them.
    for _ in 0..RAPID_PROBE_COUNT {
        send_probe(
            net_conn,
            local_mac,
            &cluster_id_raw,
            &node_id,
            own_service_name,
            &cluster.info,
        );
        sleep_ms(RAPID_PROBE_INTERVAL_MS);
    }

    let mut next_background_probe_ms: u64 =
        RAPID_PROBE_COUNT as u64 * RAPID_PROBE_INTERVAL_MS + BACKGROUND_PROBE_INTERVAL_MS;
    let mut tick_ms: u64 = RAPID_PROBE_COUNT as u64 * RAPID_PROBE_INTERVAL_MS;
    let mut heart: u32 = 0;

    loop {
        heart = heart.wrapping_add(1);
        heartbeat(heart);

        // Refresh the local cluster posture (raft role + leader hint) on a
        // slow cadence; the result is advertised in probes and responses and
        // reported by OP_CLUSTER_STATUS.
        cluster.maybe_start(ns_conn, &node_id, tick_ms);
        cluster.poll();
        config::write::<u32>(44, cluster.info.role as u32);

        // Process endpoint messages (non-blocking drain): control ops and
        // OP_FRAME ingress from the frouter.
        loop {
            let message = ipc_recv(ep);
            if message.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() };
            }
            if message.status == ipc_status::NO_MESSAGE {
                break;
            }
            if message.status == ipc_status::OK && message.reply != 0 {
                let served = unsafe { DIAG_RX_RAW };
                if served < 8 {
                    catten_syscall::el0_log(
                        0x4449_5343,
                        0x4000 | (message.opcode as u64) | ((served as u64) << 24),
                    );
                }
            }
            if !message.is_ok() || message.reply == 0 {
                continue;
            }
            match message.opcode {
                disco::OP_FRAME => {
                    unsafe { DIAG_RX_RAW = DIAG_RX_RAW.wrapping_add(1) };
                    let frame_len = message.arg0 as usize;
                    if message.memory == 0 || !(FRAME_HEADER_SIZE..=4096).contains(&frame_len) {
                        if message.memory != 0 {
                            memory_close(message.memory);
                        }
                        ipc_reply(message.reply, -1);
                        continue;
                    }
        let (rx_scratch_map_status, rx_scratch_vaddr) = memory_map_any(message.memory, false);
                    if rx_scratch_map_status == 0 {
                        let frame = unsafe {
                            core::slice::from_raw_parts(rx_scratch_vaddr as *const u8, frame_len)
                        };
                        handle_frame(
                            net_conn,
                            local_mac,
                            &cluster_id_raw,
                            &node_id,
                            own_service_name,
                            &cluster.info,
                            &mut peers,
                            tick_ms,
                            frame,
                        );
                        memory_unmap(message.memory);
                    }
                    memory_close(message.memory);
                    ipc_reply(message.reply, 0);
                }
                disco::OP_PROBE => {
                    send_probe(
                        net_conn,
                        local_mac,
                        &cluster_id_raw,
                        &node_id,
                        own_service_name,
                        &cluster.info,
                    );
                    let count = peers.len() as i64;
                    ipc_reply(message.reply, count);
                }
                disco::OP_LIST_PEERS => {
                    let count = peers.len() as u32;
                    config::write::<u32>(4, count);
                    // Reply with the packed peer list: count:u32, then per
                    // peer { mac:[u8;6], node_id_len:u8, node_id }.
                    let mut buf = alloc::vec![0u8; 4];
                    buf[0..4].copy_from_slice(&count.to_le_bytes());
                    for (mac, peer) in peers.iter() {
                        buf.extend_from_slice(mac);
                        buf.push(peer.node_id.len().min(255) as u8);
                        buf.extend_from_slice(&peer.node_id[..peer.node_id.len().min(255)]);
                    }
                    let cap = memory_alloc(1);
        let (list_scratch_2_map_status, list_scratch_2_vaddr) = memory_map_any(cap, true);
                    if cap != 0 && list_scratch_2_map_status == 0 {
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                buf.as_ptr(),
                                list_scratch_2_vaddr as *mut u8,
                                buf.len(),
                            );
                        }
                        memory_unmap(cap);
                        ipc_reply_move(message.reply, cap, buf.len() as i64);
                    } else {
                        if cap != 0 {
                            memory_close(cap);
                        }
                        ipc_reply(message.reply, -1);
                    }
                }
                disco::OP_CLUSTER_STATUS => {
                    catten_syscall::el0_log(0x4449_5343, 0x7777);
                    let mut buf = [0u8; 512];
                    let peers_vec: Vec<PeerClusterInfo<'_>> = peers
                        .iter()
                        .map(|(mac, peer)| PeerClusterInfo {
                            mac: *mac,
                            role: peer.role,
                            raft_id: &peer.raft_id,
                            leader_id: &peer.leader_id,
                        })
                        .collect();
                    let len = build_cluster_answer(
                        &mut buf,
                        cluster.info.role,
                        &cluster.info.raft_id,
                        &cluster.info.leader_id,
                        &peers_vec,
                    );
                    if let Some(len) = len {
                        let cap = memory_alloc(1);
        let (list_scratch_map_status, list_scratch_vaddr) = memory_map_any(cap, true);
                        if cap != 0 && list_scratch_map_status == 0 {
                            unsafe {
                                core::ptr::copy_nonoverlapping(
                                    buf.as_ptr(),
                                    list_scratch_vaddr as *mut u8,
                                    len,
                                );
                            }
                            memory_unmap(cap);
                            ipc_reply_move(message.reply, cap, len as i64);
                        } else {
                            if cap != 0 {
                                memory_close(cap);
                            }
                            ipc_reply(message.reply, -1);
                        }
                    } else {
                        ipc_reply(message.reply, -1);
                    }
                }
                disco::OP_STATUS => {
                    let running: u64 = 1;
                    let count = peers.len() as u64;
                    ipc_reply(message.reply, ((running) | (count << 8)) as i64);
                }
                disco::OP_SHUTDOWN => {
                    ipc_reply(message.reply, 0);
                    unsafe { thread_exit() };
                }
                _ => {
                    ipc_reply(message.reply, -1);
                }
            }
        }

        // Background probe: rebroadcast so nodes that boot later are learned.
        if tick_ms >= next_background_probe_ms {
            send_probe(
                net_conn,
                local_mac,
                &cluster_id_raw,
                &node_id,
                own_service_name,
                &cluster.info,
            );
            next_background_probe_ms = tick_ms + BACKGROUND_PROBE_INTERVAL_MS;
        }

        // Evict peers whose TTL expired.
        evict_expired(&mut peers, tick_ms);

        // Wait for an endpoint message (a discovery frame from the frouter, a
        // control call) or the next background-probe deadline.
        let sleep_ms = next_background_probe_ms.saturating_sub(tick_ms).max(1);
        let (_, timed_out) = cq_wait_timeout(1, sleep_ms, 0);
        if timed_out != 0 {
            tick_ms = next_background_probe_ms;
        } else {
            tick_ms = tick_ms.saturating_add(1);
        }
        publish_diag();
    }
}

catten_rt::entry!(main);
