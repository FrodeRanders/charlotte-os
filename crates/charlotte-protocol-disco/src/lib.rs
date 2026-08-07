//! `charlotte-protocol-disco` — the cluster discovery protocol v1.
//!
//! Nodes broadcast a probe on EtherType `0x88B6`; listening nodes reply with
//! unicast responses. Reliability comes from probe retransmission; no
//! sequencing or ACKing is needed — matching the pattern used by mDNS, SSDP,
//! and LLDP.
//!
//! ## Wire format (Ethernet frame on EtherType 0x88B6)
//!
//! ```text
//!  0..6   Destination MAC (ff:ff:ff:ff:ff:ff for probe, unicast for response)
//!  6..12  Source MAC
//! 12..14  EtherType = 0x88B6 (big-endian / network byte order)
//! 14..16  Version = 1 (u16 BE)
//! 16..18  Flags: bit 0 = PROBE, bit 1 = RESPONSE (u16 BE)
//! 18..24  Source MAC duplicated (6 bytes)
//! 24..26  Reserved (2 bytes, zero)
//! 26..42  Cluster ID (16 bytes)
//! 42..    Payload (variable, only present in RESPONSE frames)
//! ```
//!
//! **Probe:** flags = PROBE. Sent to `ff:ff:ff:ff:ff:ff`. Payload is empty.
//!
//! **Response:** flags = RESPONSE. Unicast to the probe's source MAC. Payload:
//! ```text
//!  0..1   Node ID length (u8, up to 63)
//!  1..N   Node ID (UTF-8 bytes)
//!  N..N+8 Service name (u64 LE)
//!  N+8    Cluster role (u8: 0 = not in a cluster, 1 = follower,
//!         2 = candidate, 3 = leader, 0xff = unknown/legacy)
//!  +1     Own raft id length (u8)
//!  ..     Own raft id (UTF-8 bytes)
//!  +1     Known leader's raft id length (u8)
//!  ..     Known leader's raft id (UTF-8 bytes)
//! ```
//!
//! The trailing cluster block is optional: a legacy v1 responder omits it and
//! the peer reports role `UNKNOWN`. The cluster block lets discovery answer
//! "which node leads the cluster (if any)" so a joining node can contact the
//! leader directly, or a follower/observer that redirects, or receive the
//! honest "not in a cluster" answer.
#![no_std]

extern crate alloc;

pub const DISCO_ETHERTYPE: u16 = 0x88b6;
pub const DISCO_VERSION: u16 = 1;
pub const FLAG_PROBE: u16 = 1 << 0;
pub const FLAG_RESPONSE: u16 = 1 << 1;

pub const ETHERNET_HEADER_SIZE: usize = 14;
pub const DISCO_HEADER_SIZE: usize = 28;
pub const FRAME_HEADER_SIZE: usize = ETHERNET_HEADER_SIZE + DISCO_HEADER_SIZE; // 42

pub const CLUSTER_ID_LEN: usize = 16;
pub const MAX_NODE_ID_LEN: usize = 63;
pub const MAX_SERVICE_NAME_LEN: usize = 8;

pub const BROADCAST_MAC: [u8; 6] = [0xff; 6];

pub const DISCO_INTERFACE: u64 = u64::from_le_bytes(*b"DISC O\0\0");
pub const DISCO_NAME: u64 = u64::from_le_bytes(*b"disco\0\0\0");

pub const OP_PROBE: u32 = 1;
pub const OP_LIST_PEERS: u32 = 2;
pub const OP_STATUS: u32 = 3;
pub const OP_SHUTDOWN: u32 = 4;
/// Cluster-location query: reply moves a packed blob (see
/// [`build_cluster_answer`]) describing this node's cluster role, its raft
/// id, the known leader's raft id, and every discovered peer's role/raft id.
/// This is the "where do I join" answer a joining node asks for.
pub const OP_CLUSTER_STATUS: u32 = 6;

pub const PROBE_COUNT: usize = 3;
pub const PROBE_INTERVAL_MS: u64 = 200;
pub const PEER_TTL_MS: u64 = 30_000;

/// Cluster-role values carried in the discovery payload.
pub const ROLE_NO_CLUSTER: u8 = 0;
pub const ROLE_FOLLOWER: u8 = 1;
pub const ROLE_CANDIDATE: u8 = 2;
pub const ROLE_LEADER: u8 = 3;
/// A legacy peer whose response predates the cluster block.
pub const ROLE_UNKNOWN: u8 = 0xff;

/// Build a full discovery frame header (42 bytes: 14 Ethernet + 28 disco).
pub fn build_disco_frame(
    buf: &mut [u8; FRAME_HEADER_SIZE],
    dst_mac: [u8; 6],
    src_mac: [u8; 6],
    flags: u16,
    cluster_id: &[u8; CLUSTER_ID_LEN],
) {
    buf[..6].copy_from_slice(&dst_mac);
    buf[6..12].copy_from_slice(&src_mac);
    buf[12..14].copy_from_slice(&DISCO_ETHERTYPE.to_be_bytes());
    buf[14..16].copy_from_slice(&DISCO_VERSION.to_be_bytes());
    buf[16..18].copy_from_slice(&flags.to_be_bytes());
    buf[18..24].copy_from_slice(&src_mac);
    buf[24..26].copy_from_slice(&[0, 0]);
    buf[26..42].copy_from_slice(cluster_id);
}

/// Build a response payload into `buf`. Returns the number of bytes written.
/// Returns 0 if `node_id` is empty or too long.
pub fn build_response_payload(buf: &mut [u8; 256], node_id: &[u8], service_name: u64) -> usize {
    if node_id.is_empty() || node_id.len() > MAX_NODE_ID_LEN {
        return 0;
    }
    buf[0] = node_id.len() as u8;
    let id_len = node_id.len();
    buf[1..1 + id_len].copy_from_slice(node_id);
    let name_bytes = service_name.to_le_bytes();
    buf[1 + id_len..1 + id_len + 8].copy_from_slice(&name_bytes);
    1 + id_len + 8
}

/// Parse a response payload. Returns `None` if the payload is malformed.
pub fn parse_response_payload(payload: &[u8]) -> Option<(&[u8], u64)> {
    if payload.is_empty() {
        return None;
    }
    let id_len = payload[0] as usize;
    if id_len == 0 || id_len > MAX_NODE_ID_LEN {
        return None;
    }
    let total = 1 + id_len + 8;
    if payload.len() < total {
        return None;
    }
    let node_id = &payload[1..1 + id_len];
    let service_name = u64::from_le_bytes(payload[1 + id_len..total].try_into().ok()?);
    Some((node_id, service_name))
}

/// Build an extended response payload: the v1 `{node_id, service_name}`
/// block plus the optional cluster block `{role, raft_id, leader_id}`.
/// Returns the written length (the v1 length if the cluster block does not
/// fit the 256-byte budget, so the frame stays parseable by legacy peers).
pub fn build_extended_payload(
    buf: &mut [u8; 256],
    node_id: &[u8],
    service_name: u64,
    role: u8,
    raft_id: &[u8],
    leader_id: &[u8],
) -> usize {
    let base = build_response_payload(buf, node_id, service_name);
    if base == 0 || raft_id.len() > 255 || leader_id.len() > 255 {
        return base;
    }
    if buf.len() < base + 1 + 1 + raft_id.len() + 1 + leader_id.len() {
        return base;
    }
    let mut pos = base;
    buf[pos] = role;
    pos += 1;
    buf[pos] = raft_id.len() as u8;
    pos += 1;
    buf[pos..pos + raft_id.len()].copy_from_slice(raft_id);
    pos += raft_id.len();
    buf[pos] = leader_id.len() as u8;
    pos += 1;
    buf[pos..pos + leader_id.len()].copy_from_slice(leader_id);
    pos + leader_id.len()
}

/// Parse an extended response payload into
/// `(node_id, service_name, Option<(role, raft_id, leader_id)>)` where the
/// cluster block is `None` for legacy v1 payloads.
pub fn parse_extended_payload(payload: &[u8]) -> Option<(&[u8], u64, Option<(u8, &[u8], &[u8])>)> {
    let (node_id, service_name) = parse_response_payload(payload)?;
    let base = 1 + node_id.len() + 8;
    let rest = &payload[base..];
    if rest.len() >= 2 {
        let role = rest[0];
        let raft_len = rest[1] as usize;
        if rest.len() >= 2 + raft_len + 1 {
            let leader_len = rest[2 + raft_len] as usize;
            if rest.len() >= 2 + raft_len + 1 + leader_len {
                return Some((
                    node_id,
                    service_name,
                    Some((
                        role,
                        &rest[2..2 + raft_len],
                        &rest[3 + raft_len..3 + raft_len + leader_len],
                    )),
                ));
            }
        }
    }
    Some((node_id, service_name, None))
}

/// Build the packed `OP_CLUSTER_STATUS` reply into `buf`:
/// `[0]` self role, `[1]` self raft-id length, raft id bytes, then
/// self leader-id length + bytes, then u32 peer count, then per peer
/// `{ mac[6], role:1, raft_id_len:1, raft_id }`. Returns the written
/// length, or `None` if the buffer is too small.
pub fn build_cluster_answer(
    buf: &mut [u8],
    self_role: u8,
    self_raft_id: &[u8],
    self_leader_id: &[u8],
    peers: &[PeerClusterInfo],
) -> Option<usize> {
    if self_raft_id.len() > 255 || self_leader_id.len() > 255 {
        return None;
    }
    let mut pos = 0usize;
    buf[pos] = self_role;
    pos += 1;
    buf[pos] = self_raft_id.len() as u8;
    pos += 1;
    buf[pos..pos + self_raft_id.len()].copy_from_slice(self_raft_id);
    pos += self_raft_id.len();
    buf[pos] = self_leader_id.len() as u8;
    pos += 1;
    buf[pos..pos + self_leader_id.len()].copy_from_slice(self_leader_id);
    pos += self_leader_id.len();
    if buf.len() < pos + 4 {
        return None;
    }
    buf[pos..pos + 4].copy_from_slice(&(peers.len() as u32).to_le_bytes());
    pos += 4;
    for peer in peers {
        if peer.raft_id.len() > 255
            || peer.leader_id.len() > 255
            || buf.len() < pos + 9 + peer.raft_id.len() + peer.leader_id.len()
        {
            return None;
        }
        buf[pos..pos + 6].copy_from_slice(&peer.mac);
        pos += 6;
        buf[pos] = peer.role;
        pos += 1;
        buf[pos] = peer.raft_id.len() as u8;
        pos += 1;
        buf[pos..pos + peer.raft_id.len()].copy_from_slice(&peer.raft_id);
        pos += peer.raft_id.len();
        buf[pos] = peer.leader_id.len() as u8;
        pos += 1;
        buf[pos..pos + peer.leader_id.len()].copy_from_slice(&peer.leader_id);
        pos += peer.leader_id.len();
    }
    Some(pos)
}

/// A discovered peer's cluster posture, as carried in a cluster answer.
pub struct PeerClusterInfo<'a> {
    pub mac: [u8; 6],
    pub role: u8,
    pub raft_id: &'a [u8],
    /// The cluster leader this peer redirects towards, if any.
    pub leader_id: &'a [u8],
}

/// Parse a packed `OP_CLUSTER_STATUS` reply into `(self_role, self_raft_id,
/// self_leader_id, peers)` where each peer is `(mac, role, raft_id,
/// leader_id)`.
pub fn parse_cluster_answer(
    bytes: &[u8],
) -> Option<(
    u8,
    &[u8],
    &[u8],
    alloc::vec::Vec<([u8; 6], u8, alloc::vec::Vec<u8>, alloc::vec::Vec<u8>)>,
)> {
    let mut pos = 0usize;
    let self_role = *bytes.get(pos)?;
    pos += 1;
    let raft_len = *bytes.get(pos)? as usize;
    pos += 1;
    let self_raft_id = bytes.get(pos..pos + raft_len)?;
    pos += raft_len;
    let leader_len = *bytes.get(pos)? as usize;
    pos += 1;
    let self_leader_id = bytes.get(pos..pos + leader_len)?;
    pos += leader_len;
    let count = u32::from_le_bytes(bytes.get(pos..pos + 4)?.try_into().ok()?) as usize;
    pos += 4;
    let mut peers = alloc::vec::Vec::new();
    for _ in 0..count {
        let mac: [u8; 6] = bytes.get(pos..pos + 6)?.try_into().ok()?;
        pos += 6;
        let role = *bytes.get(pos)?;
        pos += 1;
        let raft_len = *bytes.get(pos)? as usize;
        pos += 1;
        let raft_id = bytes.get(pos..pos + raft_len)?.to_vec();
        pos += raft_len;
        let leader_len = *bytes.get(pos)? as usize;
        pos += 1;
        let leader_id = bytes.get(pos..pos + leader_len)?.to_vec();
        pos += leader_len;
        peers.push((mac, role, raft_id, leader_id));
    }
    Some((self_role, self_raft_id, self_leader_id, peers))
}

/// Parse a discovery frame header from a raw Ethernet frame. Returns
/// `Some((version, flags, source_mac, cluster_id))` on success, or `None` if
/// the frame is too short or has an unexpected EtherType.
pub fn parse_disco_frame(frame: &[u8]) -> Option<(u16, u16, [u8; 6], [u8; CLUSTER_ID_LEN])> {
    if frame.len() < FRAME_HEADER_SIZE {
        return None;
    }
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    if ethertype != DISCO_ETHERTYPE {
        return None;
    }
    let version = u16::from_be_bytes([frame[14], frame[15]]);
    let flags = u16::from_be_bytes([frame[16], frame[17]]);
    let source_mac: [u8; 6] = frame[18..24].try_into().ok()?;
    let cluster_id: [u8; CLUSTER_ID_LEN] = frame[26..42].try_into().ok()?;
    Some((version, flags, source_mac, cluster_id))
}

/// Parse the peer list returned by the disco service's `OP_LIST_PEERS`:
/// `count:u32`, then per peer `{ mac:[u8;6], node_id_len:u8, node_id }`.
pub fn parse_peer_list(bytes: &[u8]) -> alloc::vec::Vec<([u8; 6], alloc::vec::Vec<u8>)> {
    let mut peers = alloc::vec::Vec::new();
    if bytes.len() < 4 {
        return peers;
    }
    let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let mut pos = 4;
    for _ in 0..count {
        if bytes.len() < pos + 7 {
            break;
        }
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&bytes[pos..pos + 6]);
        pos += 6;
        let id_len = bytes[pos] as usize;
        pos += 1;
        if bytes.len() < pos + id_len {
            break;
        }
        let node_id = bytes[pos..pos + id_len].to_vec();
        pos += id_len;
        peers.push((mac, node_id));
    }
    peers
}
