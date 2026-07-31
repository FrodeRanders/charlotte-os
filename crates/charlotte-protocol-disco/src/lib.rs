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
//! ```
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

pub const PROBE_COUNT: usize = 3;
pub const PROBE_INTERVAL_MS: u64 = 200;
pub const PEER_TTL_MS: u64 = 30_000;

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
