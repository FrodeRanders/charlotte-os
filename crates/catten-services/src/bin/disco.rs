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
    sleep_ms,
    wait_for_boot_done,
    wait_reply,
};
use catten_syscall::{
    IpcRights,
    cq_wait_timeout,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_scalar_call_move,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map,
    memory_unmap,
    thread_exit,
};
use charlotte_protocol_disco::{
    BROADCAST_MAC,
    CLUSTER_ID_LEN,
    FLAG_PROBE,
    FLAG_RESPONSE,
    FRAME_HEADER_SIZE,
    build_disco_frame,
    build_response_payload,
    parse_disco_frame,
    parse_response_payload,
};
use charlotte_protocol_net::decode_status;

const REPLY_SPINS: u64 = 50_000_000;
const TX_SCRATCH: usize = 0x0000_0000_0090_0000;
const RX_SCRATCH: usize = 0x0000_0000_0090_1000;
const LIST_SCRATCH: usize = 0x0000_0000_0090_2000;

const RAPID_PROBE_COUNT: usize = 3;
const RAPID_PROBE_INTERVAL_MS: u64 = 200;
const BACKGROUND_PROBE_INTERVAL_MS: u64 = 15_000;

#[allow(dead_code)]
struct DiscoveredPeer {
    mac: [u8; 6],
    node_id: Vec<u8>,
    service_name: u64,
    deadline_ms: u64,
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
    if memory_map(cap, TX_SCRATCH, true) != 0 {
        memory_close(cap);
        return false;
    }
    config::write::<u32>(40, 3);
    unsafe {
        core::ptr::copy_nonoverlapping(frame.as_ptr(), TX_SCRATCH as *mut u8, frame.len());
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
) {
    let mut payload_buf = [0u8; 256];
    let payload_len = build_response_payload(&mut payload_buf, node_id, service_name);
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
) {
    let mut payload_buf = [0u8; 256];
    let payload_len = build_response_payload(&mut payload_buf, node_id, service_name);
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
    peers: &mut BTreeMap<[u8; 6], DiscoveredPeer>,
    tick_ms: u64,
    frame: &[u8],
) {
    if let Some((_version, flags, source_mac, frame_cluster_id)) = parse_disco_frame(frame) {
        unsafe { DIAG_DECODED = DIAG_DECODED.wrapping_add(1) };
        if frame_cluster_id == *cluster_id && source_mac != local_mac {
            let payload = &frame[FRAME_HEADER_SIZE..];
            if let Some((peer_id, service_name)) = parse_response_payload(payload) {
                learn_peer(
                    peers,
                    source_mac,
                    peer_id,
                    service_name,
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

    // Wait until this node has finished booting before broadcasting, so the
    // NIC and the two-node socket transport have settled. Probes sent during
    // the boot storm are silently lost and never retried.
    if !wait_for_boot_done(ns_conn) {
        config::write::<u32>(0, 0xff10);
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 6);

    // Send rapid bootstrap probes before entering the reactor. Subsequent
    // background probes are paced inside the loop so a blocked receive can
    // never starve them.
    for _ in 0..RAPID_PROBE_COUNT {
        send_probe(net_conn, local_mac, &cluster_id_raw, &node_id, own_service_name);
        sleep_ms(RAPID_PROBE_INTERVAL_MS);
    }

    let mut next_background_probe_ms: u64 =
        RAPID_PROBE_COUNT as u64 * RAPID_PROBE_INTERVAL_MS + BACKGROUND_PROBE_INTERVAL_MS;
    let mut tick_ms: u64 = RAPID_PROBE_COUNT as u64 * RAPID_PROBE_INTERVAL_MS;
    let mut heart: u32 = 0;

    loop {
        heart = heart.wrapping_add(1);
        heartbeat(heart);

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
                    unsafe { DIAG_RX_RAW = DIAG_RX_RAW.wrapping_add(1) };
                    let frame_len = message.arg0 as usize;
                    if message.memory == 0
                        || !(FRAME_HEADER_SIZE..=4096).contains(&frame_len)
                    {
                        if message.memory != 0 {
                            memory_close(message.memory);
                        }
                        ipc_reply(message.reply, -1);
                        continue;
                    }
                    if memory_map(message.memory, RX_SCRATCH, false) == 0 {
                        let frame = unsafe {
                            core::slice::from_raw_parts(RX_SCRATCH as *const u8, frame_len)
                        };
                        handle_frame(
                            net_conn,
                            local_mac,
                            &cluster_id_raw,
                            &node_id,
                            own_service_name,
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
                    send_probe(net_conn, local_mac, &cluster_id_raw, &node_id, own_service_name);
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
                    if cap != 0 && memory_map(cap, LIST_SCRATCH, true) == 0 {
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                buf.as_ptr(),
                                LIST_SCRATCH as *mut u8,
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
            send_probe(net_conn, local_mac, &cluster_id_raw, &node_id, own_service_name);
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
