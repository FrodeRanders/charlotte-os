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
use core::sync::atomic::{
    AtomicU32,
    Ordering,
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
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_scalar_call_move,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
    submit_detached_timer,
    thread_exit,
};
use charlotte_launch::disco_status as status;
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
    cluster_answer_len,
    parse_disco_frame,
    parse_extended_payload,
};
use charlotte_protocol_net::decode_status;

const RAPID_PROBE_COUNT: usize = 3;
const RAPID_PROBE_INTERVAL_MS: u64 = 200;
const BACKGROUND_PROBE_INTERVAL_MS: u64 = 15_000;
const RAFT_STATUS_REFRESH_MS: u64 = 2_000;
const CLOCK_TICK_MS: u64 = 100;
const CLOCK_TIMER_COOKIE: u64 = 0x4449_5343_5449_434b;

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
                if generation >= 1 && conn != 0 {
                    self.status_conn = conn;
                    self.pending_status = ipc_scalar_call(conn, raft::OP_CLUSTER_STATUS, 0);
                    if self.pending_status == 0 {
                        ipc_close(self.status_conn);
                        self.status_conn = 0;
                        changed |= self.set_not_in_cluster();
                    }
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
            let (status, _result, conn, memory) = ipc_reply_poll_with_memory(self.pending_status);
            if status == 0 {
                ipc_close(self.pending_status);
                self.pending_status = 0;
                if self.status_conn != 0 {
                    ipc_close(self.status_conn);
                    self.status_conn = 0;
                }
                if conn != 0 {
                    ipc_close(conn);
                }
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
                if conn != 0 {
                    ipc_close(conn);
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
            let bytes =
                unsafe { core::slice::from_raw_parts(rstat_scratch_vaddr as *const u8, 4096) };
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

static DIAG_RX_RAW: AtomicU32 = AtomicU32::new(0);
static DIAG_SENT_OK: AtomicU32 = AtomicU32::new(0);
static DIAG_SENT_FAIL: AtomicU32 = AtomicU32::new(0);
static DIAG_DECODED: AtomicU32 = AtomicU32::new(0);
static DIAG_CALLED: AtomicU32 = AtomicU32::new(0);

fn publish_diag() {
    config::write::<u32>(status::RX_RAW, DIAG_RX_RAW.load(Ordering::Relaxed));
    config::write::<u32>(status::SENT_OK, DIAG_SENT_OK.load(Ordering::Relaxed));
    config::write::<u32>(status::SENT_FAIL, DIAG_SENT_FAIL.load(Ordering::Relaxed));
    config::write::<u32>(status::DECODED, DIAG_DECODED.load(Ordering::Relaxed));
    config::write::<u32>(status::CALLED, DIAG_CALLED.load(Ordering::Relaxed));
}

fn heartbeat(beat: u32) {
    config::write::<u32>(status::HEARTBEAT, beat);
}

fn cluster_posture_ready(cluster: &ClusterInfo, peers: &BTreeMap<[u8; 6], DiscoveredPeer>) -> bool {
    !cluster.raft_id.is_empty()
        && peers.values().any(|peer| !peer.raft_id.is_empty() && peer.raft_id != cluster.raft_id)
}

fn reply_cluster_status(
    reply: u64,
    cluster: &ClusterInfo,
    peers: &BTreeMap<[u8; 6], DiscoveredPeer>,
) {
    let peers_vec: Vec<PeerClusterInfo<'_>> = peers
        .iter()
        .map(|(mac, peer)| PeerClusterInfo {
            mac: *mac,
            role: peer.role,
            raft_id: &peer.raft_id,
            leader_id: &peer.leader_id,
        })
        .collect();
    let Some(len) = cluster_answer_len(&cluster.raft_id, &cluster.leader_id, &peers_vec) else {
        ipc_reply(reply, -1);
        return;
    };
    let mut buf = alloc::vec![0u8; len];
    let encoded = build_cluster_answer(
        &mut buf,
        cluster.role,
        &cluster.raft_id,
        &cluster.leader_id,
        &peers_vec,
    );
    let pages = len.div_ceil(4096).max(1);
    let cap = memory_alloc(pages);
    if cap == 0 {
        ipc_reply(reply, -1);
        return;
    }
    let (map_status, vaddr) = memory_map_any(cap, true);
    if encoded == Some(len) && map_status == 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), vaddr as *mut u8, len);
        }
        memory_unmap(cap);
        if ipc_reply_move(reply, cap, len as i64) != 0 {
            memory_close(cap);
        }
    } else {
        memory_close(cap);
        ipc_reply(reply, -1);
    }
}

fn send_raw_frame(net_conn: u64, frame: &[u8]) -> bool {
    if frame.len() > 4096 {
        return false;
    }
    config::write::<u32>(status::SEND_PROGRESS, 1);
    let cap = memory_alloc(1);
    if cap == 0 {
        return false;
    }
    config::write::<u32>(status::SEND_PROGRESS, 2);
    let (tx_scratch_map_status, tx_scratch_vaddr) = memory_map_any(cap, true);
    if tx_scratch_map_status != 0 {
        memory_close(cap);
        return false;
    }
    config::write::<u32>(status::SEND_PROGRESS, 3);
    unsafe {
        core::ptr::copy_nonoverlapping(frame.as_ptr(), tx_scratch_vaddr as *mut u8, frame.len());
    }
    memory_unmap(cap);
    let call = ipc_scalar_call_move(net_conn, net::OP_SEND, frame.len() as u64, cap);
    if call == 0 {
        memory_close(cap);
        DIAG_SENT_FAIL.fetch_add(1, Ordering::Relaxed);
        config::write::<u32>(status::SEND_PROGRESS, 0xff);
        return false;
    }
    DIAG_CALLED.fetch_add(1, Ordering::Relaxed);
    config::write::<u32>(status::SEND_PROGRESS, 4);
    let (result, returned_cap) = unsafe { wait_reply(call, 0) };
    if returned_cap != 0 {
        ipc_close(returned_cap);
    }
    config::write::<u32>(status::SEND_PROGRESS, 5);
    if result == 0 {
        DIAG_SENT_OK.fetch_add(1, Ordering::Relaxed);
        true
    } else {
        DIAG_SENT_FAIL.fetch_add(1, Ordering::Relaxed);
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

#[allow(clippy::too_many_arguments)]
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
    config::write::<u32>(status::PEER_COUNT, peers.len() as u32);
}

fn evict_expired(peers: &mut BTreeMap<[u8; 6], DiscoveredPeer>, tick_ms: u64) {
    let expired: Vec<[u8; 6]> =
        peers.iter().filter(|(_, p)| tick_ms > p.deadline_ms).map(|(m, _)| *m).collect();
    if !expired.is_empty() {
        for mac in expired {
            peers.remove(&mac);
        }
        config::write::<u32>(status::PEER_COUNT, peers.len() as u32);
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
        DIAG_DECODED.fetch_add(1, Ordering::Relaxed);
        if frame_cluster_id == *cluster_id && source_mac != local_mac {
            let payload = &frame[FRAME_HEADER_SIZE..];
            if let Some((peer_id, service_name, cluster_block)) = parse_extended_payload(payload) {
                let (role, raft_id, leader_id) = cluster_block.unwrap_or((ROLE_UNKNOWN, &[], &[]));
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
    config::write::<u32>(status::STAGE, 1);
    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 2);

    let lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, net::NAME);
    if lookup == 0 {
        unsafe { thread_exit() };
    }
    let (generation, net_conn) = unsafe { wait_reply(lookup, 0) };
    if generation < 1 || net_conn == 0 {
        if net_conn != 0 {
            ipc_close(net_conn);
        }
        unsafe { thread_exit() };
    }

    let status_call = ipc_scalar_call(net_conn, net::OP_STATUS, 0);
    if status_call == 0 {
        unsafe { thread_exit() };
    }
    let (status, status_cap) = unsafe { wait_reply(status_call, 0) };
    if status_cap != 0 {
        ipc_close(status_cap);
    }
    let (link, local_mac) = decode_status(status);
    if link == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 3);

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
            config::write::<u32>(status::STAGE, 0xff03);
            unsafe { thread_exit() };
        }
    };
    config::write::<u32>(status::STAGE, 4);

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
    let (reg_gen, registration_cap) = unsafe { wait_reply(registration, 0) };
    if registration_cap != 0 {
        ipc_close(registration_cap);
    }
    if reg_gen < 1 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 5);
    publish_diag();

    let own_service_name = disco::NAME;
    let mut peers: BTreeMap<[u8; 6], DiscoveredPeer> = BTreeMap::new();
    let mut cluster = ClusterProbe::new();
    let mut cluster_waiters: Vec<u64> = Vec::new();

    // Wait until this node has finished booting before broadcasting, so the
    // NIC and the two-node socket transport have settled. Probes sent during
    // the boot storm are silently lost and never retried.
    if !wait_for_local_ready(ns_conn) {
        config::write::<u32>(status::STAGE, 0xff10);
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 6);

    // Send rapid bootstrap probes before entering the reactor. Subsequent
    // background probes are paced inside the loop so a blocked receive can
    // never starve them.
    for _ in 0..RAPID_PROBE_COUNT {
        send_probe(net_conn, local_mac, &cluster_id_raw, &node_id, own_service_name, &cluster.info);
        sleep_ms(RAPID_PROBE_INTERVAL_MS);
    }

    let mut next_background_probe_ms: u64 =
        RAPID_PROBE_COUNT as u64 * RAPID_PROBE_INTERVAL_MS + BACKGROUND_PROBE_INTERVAL_MS;
    let mut tick_ms: u64 = RAPID_PROBE_COUNT as u64 * RAPID_PROBE_INTERVAL_MS;
    let cq = ctx.completion_queue_layout();
    let mut clock_armed = submit_detached_timer(CLOCK_TICK_MS, 0, CLOCK_TIMER_COOKIE) != u64::MAX;
    let mut heart: u32 = 0;

    loop {
        heart = heart.wrapping_add(1);
        heartbeat(heart);

        // Refresh the local cluster posture (raft role + leader hint) on a
        // slow cadence; the result is advertised in probes and responses and
        // reported by OP_CLUSTER_STATUS.
        cluster.maybe_start(ns_conn, &node_id, tick_ms);
        cluster.poll();
        config::write::<u32>(status::CLUSTER_ROLE, cluster.info.role as u32);

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
            if !message.is_ok() || message.reply == 0 {
                continue;
            }
            match message.opcode {
                disco::OP_FRAME => {
                    DIAG_RX_RAW.fetch_add(1, Ordering::Relaxed);
                    let frame_len = message.arg0 as usize;
                    if message.memory == 0 || !(FRAME_HEADER_SIZE..=4096).contains(&frame_len) {
                        if message.memory != 0 {
                            memory_close(message.memory);
                        }
                        ipc_reply(message.reply, -1);
                        continue;
                    }
                    let (rx_scratch_map_status, rx_scratch_vaddr) =
                        memory_map_any(message.memory, false);
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
                    config::write::<u32>(status::LAST_PROBE_PEERS, count);
                    // Reply with the packed peer list: count:u32, then per
                    // peer { mac:[u8;6], node_id_len:u8, node_id }.
                    let mut buf = alloc::vec![0u8; 4];
                    buf[0..4].copy_from_slice(&count.to_le_bytes());
                    for (mac, peer) in peers.iter() {
                        buf.extend_from_slice(mac);
                        buf.push(peer.node_id.len().min(255) as u8);
                        buf.extend_from_slice(&peer.node_id[..peer.node_id.len().min(255)]);
                    }
                    let pages = buf.len().div_ceil(4096).max(1);
                    let cap = memory_alloc(pages);
                    if cap == 0 {
                        ipc_reply(message.reply, -1);
                        continue;
                    }
                    let (list_scratch_2_map_status, list_scratch_2_vaddr) =
                        memory_map_any(cap, true);
                    if list_scratch_2_map_status == 0 {
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
                        memory_close(cap);
                        ipc_reply(message.reply, -1);
                    }
                }
                disco::OP_CLUSTER_STATUS => {
                    if message.arg0 == disco::CLUSTER_STATUS_WAIT_READY
                        && !cluster_posture_ready(&cluster.info, &peers)
                    {
                        if cluster_waiters.len() < 8 {
                            cluster_waiters.push(message.reply);
                        } else {
                            ipc_reply(message.reply, -1);
                        }
                    } else {
                        reply_cluster_status(message.reply, &cluster.info, &peers);
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

        if !cluster_waiters.is_empty() && cluster_posture_ready(&cluster.info, &peers) {
            for reply in core::mem::take(&mut cluster_waiters) {
                reply_cluster_status(reply, &cluster.info, &peers);
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

        // A real timer completion advances protocol time independently of
        // endpoint traffic. Counting CQ wakes as milliseconds made TTL and
        // refresh cadence depend on packet rate.
        let (_, timed_out) = cq_wait_timeout(1, CLOCK_TICK_MS, 0);
        let mut clock_fired = false;
        while let Some(completion) = unsafe { cq_read(cq.base, cq.entries) } {
            if completion.cookie == CLOCK_TIMER_COOKIE {
                clock_fired = true;
                clock_armed = false;
            }
        }
        // The bounded CQ wait is an independent watchdog: successfully
        // submitting a detached timer does not guarantee that its completion
        // will be delivered promptly. A delayed/lost cookie must not freeze
        // peer TTLs and Raft-posture refreshes.
        if clock_fired || timed_out != 0 {
            tick_ms = tick_ms.saturating_add(CLOCK_TICK_MS);
            if !clock_armed {
                clock_armed =
                    submit_detached_timer(CLOCK_TICK_MS, 0, CLOCK_TIMER_COOKIE) != u64::MAX;
            }
        }
        publish_diag();
    }
}

catten_rt::entry!(main);
