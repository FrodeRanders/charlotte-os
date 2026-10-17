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
//! 24..26  Peer advertisement length (u16 BE, excludes Ethernet padding)
//! 26..42  Cluster ID (16 bytes)
//! 42..    Peer advertisement (variable, required in probes and responses)
//! ```
//!
//! **Probe:** flags = PROBE. Sent to `ff:ff:ff:ff:ff:ff`. Carries the sender's
//! complete advertisement for passive peer learning.
//!
//! **Response:** flags = RESPONSE. Unicast to the probe's source MAC.
//! Both frame types carry the same complete advertisement:
//! ```text
//!  0..1   Node ID length (u8, up to 63)
//!  1..N   Node ID (UTF-8 bytes)
//!  N..N+8 Service name (u64 LE)
//!  N+8    Cluster role (u8: 0 = not in a cluster, 1 = follower,
//!         2 = candidate, 3 = leader)
//!  +1     Own raft id length (u8)
//!  ..     Own raft id (UTF-8 bytes)
//!  +1     Known leader's raft id length (u8)
//!  ..     Known leader's raft id (UTF-8 bytes)
//! ```
//!
//! Cluster posture is mandatory. A node without a cluster uses role 0 and
//! empty raft/leader IDs. Base-only and truncated advertisements reject; builders
//! never fall back to an older grammar. Identifiers are UTF-8, node IDs have
//! 1–63 bytes, raft/leader IDs have at most 255 bytes, and the complete payload
//! has at most 256 bytes. The payload parser consumes exactly one record. Frame
//! parsing additionally permits zero padding to Ethernet's 60-byte minimum.
//! Discovery supplies routing hints. Identity and membership require separate authentication.
//! The header length is required: retired zero-length headers reject. This
//! prevents Ethernet padding from masquerading as omitted cluster fields.
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
pub const MAX_PEER_PAYLOAD_LEN: usize = 256;
pub const MIN_PEER_PAYLOAD_LEN: usize = 13;
/// Ethernet minimum excluding the four-byte FCS.
pub const MIN_ETHERNET_FRAME_SIZE: usize = 60;

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

/// Build a full discovery frame header (42 bytes: 14 Ethernet + 28 disco).
/// Invalid flags or advertisement length reject without changing the buffer.
pub fn build_disco_frame(
    buf: &mut [u8; FRAME_HEADER_SIZE],
    dst_mac: [u8; 6],
    src_mac: [u8; 6],
    flags: u16,
    cluster_id: &[u8; CLUSTER_ID_LEN],
    payload_len: usize,
) -> Option<()> {
    if !matches!(flags, FLAG_PROBE | FLAG_RESPONSE)
        || !(MIN_PEER_PAYLOAD_LEN..=MAX_PEER_PAYLOAD_LEN).contains(&payload_len)
    {
        return None;
    }
    buf[..6].copy_from_slice(&dst_mac);
    buf[6..12].copy_from_slice(&src_mac);
    buf[12..14].copy_from_slice(&DISCO_ETHERTYPE.to_be_bytes());
    buf[14..16].copy_from_slice(&DISCO_VERSION.to_be_bytes());
    buf[16..18].copy_from_slice(&flags.to_be_bytes());
    buf[18..24].copy_from_slice(&src_mac);
    buf[24..26].copy_from_slice(&(payload_len as u16).to_be_bytes());
    buf[26..42].copy_from_slice(cluster_id);
    Some(())
}

/// Borrowed current advertisement carried by both probes and responses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerAdvertisement<'a> {
    pub node_id: &'a [u8],
    pub service_name: u64,
    pub role: u8,
    pub raft_id: &'a [u8],
    pub leader_id: &'a [u8],
}

fn valid_role(role: u8) -> bool {
    matches!(role, ROLE_NO_CLUSTER | ROLE_FOLLOWER | ROLE_CANDIDATE | ROLE_LEADER)
}

/// Build one complete advertisement. Rejection leaves the buffer unchanged;
/// invalid identifiers, roles or an oversized payload never produce a prefix.
pub fn build_peer_payload(
    buf: &mut [u8; MAX_PEER_PAYLOAD_LEN],
    peer: PeerAdvertisement<'_>,
) -> Option<usize> {
    if peer.node_id.is_empty()
        || peer.node_id.len() > MAX_NODE_ID_LEN
        || peer.raft_id.len() > u8::MAX as usize
        || peer.leader_id.len() > u8::MAX as usize
        || !valid_role(peer.role)
        || core::str::from_utf8(peer.node_id).is_err()
        || core::str::from_utf8(peer.raft_id).is_err()
        || core::str::from_utf8(peer.leader_id).is_err()
    {
        return None;
    }
    let len = 12 + peer.node_id.len() + peer.raft_id.len() + peer.leader_id.len();
    if len > buf.len() {
        return None;
    }
    buf[0] = peer.node_id.len() as u8;
    let mut pos = 1;
    buf[pos..pos + peer.node_id.len()].copy_from_slice(peer.node_id);
    pos += peer.node_id.len();
    buf[pos..pos + 8].copy_from_slice(&peer.service_name.to_le_bytes());
    pos += 8;
    buf[pos] = peer.role;
    pos += 1;
    buf[pos] = peer.raft_id.len() as u8;
    pos += 1;
    buf[pos..pos + peer.raft_id.len()].copy_from_slice(peer.raft_id);
    pos += peer.raft_id.len();
    buf[pos] = peer.leader_id.len() as u8;
    pos += 1;
    buf[pos..pos + peer.leader_id.len()].copy_from_slice(peer.leader_id);
    Some(len)
}

/// Parse exactly one complete advertisement, without Ethernet padding.
pub fn parse_peer_payload(payload: &[u8]) -> Option<PeerAdvertisement<'_>> {
    let id_len = usize::from(*payload.first()?);
    if id_len == 0 || id_len > MAX_NODE_ID_LEN {
        return None;
    }
    let node_id = payload.get(1..1 + id_len)?;
    let mut pos = 1 + id_len;
    let service_name = u64::from_le_bytes(payload.get(pos..pos + 8)?.try_into().ok()?);
    pos += 8;
    let role = *payload.get(pos)?;
    if !valid_role(role) {
        return None;
    }
    pos += 1;
    let raft_len = usize::from(*payload.get(pos)?);
    pos += 1;
    let raft_id = payload.get(pos..pos + raft_len)?;
    pos += raft_len;
    let leader_len = usize::from(*payload.get(pos)?);
    pos += 1;
    let leader_id = payload.get(pos..pos + leader_len)?;
    pos += leader_len;
    if pos != payload.len()
        || pos > MAX_PEER_PAYLOAD_LEN
        || core::str::from_utf8(node_id).is_err()
        || core::str::from_utf8(raft_id).is_err()
        || core::str::from_utf8(leader_id).is_err()
    {
        return None;
    }
    Some(PeerAdvertisement {
        node_id,
        service_name,
        role,
        raft_id,
        leader_id,
    })
}

/// Return the exact packed `OP_CLUSTER_STATUS` size, or `None` when a count,
/// identifier length, or total size is not representable by the wire format.
pub fn cluster_answer_len(
    self_raft_id: &[u8],
    self_leader_id: &[u8],
    peers: &[PeerClusterInfo<'_>],
) -> Option<usize> {
    if self_raft_id.len() > 255 || self_leader_id.len() > 255 || u32::try_from(peers.len()).is_err()
    {
        return None;
    }
    let mut len = 2usize
        .checked_add(self_raft_id.len())?
        .checked_add(1)?
        .checked_add(self_leader_id.len())?
        .checked_add(4)?;
    for peer in peers {
        if peer.raft_id.len() > 255 || peer.leader_id.len() > 255 {
            return None;
        }
        len = len
            .checked_add(9)?
            .checked_add(peer.raft_id.len())?
            .checked_add(peer.leader_id.len())?;
    }
    Some(len)
}

/// Build the packed `OP_CLUSTER_STATUS` reply into `buf`:
/// `[0]` self role, `[1]` self raft-id length, raft id bytes, then
/// self leader-id length + bytes, then u32 peer count, then per peer
/// `{ mac[6], role:1, raft_id_len:1, raft_id }`. Returns the written
/// length, or `None` if the contents are invalid or the buffer is too small.
pub fn build_cluster_answer(
    buf: &mut [u8],
    self_role: u8,
    self_raft_id: &[u8],
    self_leader_id: &[u8],
    peers: &[PeerClusterInfo],
) -> Option<usize> {
    let encoded_len = cluster_answer_len(self_raft_id, self_leader_id, peers)?;
    if buf.len() < encoded_len {
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
    let peer_count = u32::try_from(peers.len()).ok()?;
    buf[pos..pos + 4].copy_from_slice(&peer_count.to_le_bytes());
    pos += 4;
    for peer in peers {
        buf[pos..pos + 6].copy_from_slice(&peer.mac);
        pos += 6;
        buf[pos] = peer.role;
        pos += 1;
        buf[pos] = peer.raft_id.len() as u8;
        pos += 1;
        buf[pos..pos + peer.raft_id.len()].copy_from_slice(peer.raft_id);
        pos += peer.raft_id.len();
        buf[pos] = peer.leader_id.len() as u8;
        pos += 1;
        buf[pos..pos + peer.leader_id.len()].copy_from_slice(peer.leader_id);
        pos += peer.leader_id.len();
    }
    debug_assert_eq!(pos, encoded_len);
    Some(encoded_len)
}

/// A discovered peer's cluster posture, as carried in a cluster answer.
pub struct PeerClusterInfo<'a> {
    pub mac: [u8; 6],
    pub role: u8,
    pub raft_id: &'a [u8],
    /// The cluster leader this peer redirects towards, if any.
    pub leader_id: &'a [u8],
}

/// A peer entry in a cluster answer: `(mac, role, raft_id, leader_id)`.
pub type PeerEntry = ([u8; 6], u8, alloc::vec::Vec<u8>, alloc::vec::Vec<u8>);

/// A full cluster answer: `(self_role, self_raft_id, self_leader_id, peers)`.
pub type ClusterAnswer<'a> = (u8, &'a [u8], &'a [u8], alloc::vec::Vec<PeerEntry>);

/// Parse a packed `OP_CLUSTER_STATUS` reply into a [`ClusterAnswer`].
pub fn parse_cluster_answer(bytes: &[u8]) -> Option<ClusterAnswer<'_>> {
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

/// One validated current discovery frame. The advertisement borrows the frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryFrame<'a> {
    pub flags: u16,
    pub source_mac: [u8; 6],
    pub cluster_id: [u8; CLUSTER_ID_LEN],
    pub peer: PeerAdvertisement<'a>,
}

/// Validate the header and complete advertisement before learning or responding.
/// Accept only the current version, one known flag, an explicit length and a
/// duplicated source matching Ethernet. Only zero padding to the Ethernet
/// minimum may follow the record; extra or truncated payloads reject.
pub fn parse_disco_frame(frame: &[u8]) -> Option<DiscoveryFrame<'_>> {
    if frame.len() < FRAME_HEADER_SIZE
        || u16::from_be_bytes(frame[12..14].try_into().ok()?) != DISCO_ETHERTYPE
        || u16::from_be_bytes(frame[14..16].try_into().ok()?) != DISCO_VERSION
        || frame[6..12] != frame[18..24]
    {
        return None;
    }
    let flags = u16::from_be_bytes(frame[16..18].try_into().ok()?);
    if !matches!(flags, FLAG_PROBE | FLAG_RESPONSE) {
        return None;
    }
    let payload_len = usize::from(u16::from_be_bytes(frame[24..26].try_into().ok()?));
    if !(MIN_PEER_PAYLOAD_LEN..=MAX_PEER_PAYLOAD_LEN).contains(&payload_len) {
        return None;
    }
    let record_end = FRAME_HEADER_SIZE + payload_len;
    let peer = parse_peer_payload(frame.get(FRAME_HEADER_SIZE..record_end)?)?;
    if frame.len() != record_end
        && (frame.len() != MIN_ETHERNET_FRAME_SIZE
            || record_end >= MIN_ETHERNET_FRAME_SIZE
            || frame[record_end..].iter().any(|byte| *byte != 0))
    {
        return None;
    }
    Some(DiscoveryFrame {
        flags,
        source_mac: frame[18..24].try_into().ok()?,
        cluster_id: frame[26..42].try_into().ok()?,
        peer,
    })
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

#[cfg(test)]
mod tests {
    use super::{
        BROADCAST_MAC,
        DISCO_NAME,
        FLAG_PROBE,
        FLAG_RESPONSE,
        FRAME_HEADER_SIZE,
        MAX_PEER_PAYLOAD_LEN,
        MIN_ETHERNET_FRAME_SIZE,
        PeerAdvertisement,
        PeerClusterInfo,
        ROLE_CANDIDATE,
        ROLE_FOLLOWER,
        ROLE_LEADER,
        ROLE_NO_CLUSTER,
        build_cluster_answer,
        build_disco_frame,
        build_peer_payload,
        parse_cluster_answer,
        parse_disco_frame,
        parse_peer_payload,
    };

    fn peer() -> PeerAdvertisement<'static> {
        PeerAdvertisement {
            node_id: b"n1",
            service_name: DISCO_NAME,
            role: ROLE_LEADER,
            raft_id: b"raft-n1",
            leader_id: b"raft-n1",
        }
    }

    fn frame(flags: u16, peer: PeerAdvertisement<'_>) -> alloc::vec::Vec<u8> {
        let mut buf = [0; MAX_PEER_PAYLOAD_LEN];
        let len = build_peer_payload(&mut buf, peer).unwrap();
        let mut header = [0; FRAME_HEADER_SIZE];
        build_disco_frame(&mut header, BROADCAST_MAC, [1; 6], flags, &[2; 16], len).unwrap();
        let mut frame = header.to_vec();
        frame.extend_from_slice(&buf[..len]);
        frame
    }

    #[test]
    fn complete_advertisements_round_trip_for_both_frame_types_and_all_roles() {
        for role in [ROLE_NO_CLUSTER, ROLE_FOLLOWER, ROLE_CANDIDATE, ROLE_LEADER] {
            let peer = PeerAdvertisement {
                role,
                ..peer()
            };
            let mut buf = [0; MAX_PEER_PAYLOAD_LEN];
            let len = build_peer_payload(&mut buf, peer).unwrap();
            assert_eq!(parse_peer_payload(&buf[..len]), Some(peer));
            for flags in [FLAG_PROBE, FLAG_RESPONSE] {
                let bytes = frame(flags, peer);
                let parsed = parse_disco_frame(&bytes).unwrap();
                assert_eq!(parsed.flags, flags);
                assert_eq!(parsed.source_mac, [1; 6]);
                assert_eq!(parsed.cluster_id, [2; 16]);
                assert_eq!(parsed.peer, peer);
            }
        }
    }

    #[test]
    fn base_only_truncated_and_extra_payloads_reject_without_role_fallback() {
        let mut buf = [0; MAX_PEER_PAYLOAD_LEN];
        let len = build_peer_payload(&mut buf, peer()).unwrap();
        for end in 0..len {
            assert!(parse_peer_payload(&buf[..end]).is_none(), "prefix {end}");
        }
        assert!(parse_peer_payload(&buf[..len + 1]).is_none());
        let mut bytes = buf[..len].to_vec();
        bytes[1 + peer().node_id.len() + 8] = 0xff;
        assert!(parse_peer_payload(&bytes).is_none());
        for offset in [1, 1 + peer().node_id.len() + 10, len - 1] {
            let mut bytes = buf[..len].to_vec();
            bytes[offset] = 0xff;
            assert!(parse_peer_payload(&bytes).is_none(), "identifier offset {offset}");
        }
    }

    #[test]
    fn oversized_or_invalid_advertisements_never_publish_a_base_prefix() {
        let id = [b'n'; 63];
        let raft = [b'r'; 181];
        let maximum = PeerAdvertisement {
            node_id: &id,
            raft_id: &raft,
            leader_id: b"",
            ..peer()
        };
        let mut buf = [0; MAX_PEER_PAYLOAD_LEN];
        assert_eq!(build_peer_payload(&mut buf, maximum), Some(MAX_PEER_PAYLOAD_LEN));
        assert_eq!(parse_peer_payload(&buf), Some(maximum));
        let too_long = [b'x'; 256];
        let bad_utf8 = [0xff];
        for invalid in [
            PeerAdvertisement {
                node_id: b"",
                ..peer()
            },
            PeerAdvertisement {
                node_id: &too_long[..64],
                ..peer()
            },
            PeerAdvertisement {
                raft_id: &too_long,
                ..peer()
            },
            PeerAdvertisement {
                leader_id: &too_long,
                ..peer()
            },
            PeerAdvertisement {
                leader_id: b"x",
                ..maximum
            },
            PeerAdvertisement {
                role: 0xff,
                ..peer()
            },
            PeerAdvertisement {
                role: 4,
                ..peer()
            },
            PeerAdvertisement {
                node_id: &bad_utf8,
                ..peer()
            },
            PeerAdvertisement {
                raft_id: &bad_utf8,
                ..peer()
            },
            PeerAdvertisement {
                leader_id: &bad_utf8,
                ..peer()
            },
        ] {
            let mut buf = [0xa5; MAX_PEER_PAYLOAD_LEN];
            assert_eq!(build_peer_payload(&mut buf, invalid), None);
            assert_eq!(buf, [0xa5; MAX_PEER_PAYLOAD_LEN]);
        }
    }

    #[test]
    fn frame_rejects_retired_headers_invalid_flags_lengths_and_source_substitution() {
        let canonical = frame(FLAG_PROBE, peer());
        for end in 0..canonical.len() {
            assert!(parse_disco_frame(&canonical[..end]).is_none());
        }
        for (offset, value) in
            [(12, 0), (14, 1), (15, 0), (15, 2), (17, 0), (17, 3), (17, 4), (18, 3)]
        {
            let mut bytes = canonical.clone();
            bytes[offset] = value;
            assert!(parse_disco_frame(&bytes).is_none(), "header offset {offset} value {value}");
        }
        for len in [0, 12, MAX_PEER_PAYLOAD_LEN + 1, canonical.len() - FRAME_HEADER_SIZE - 1] {
            let mut bytes = canonical.clone();
            bytes[24..26].copy_from_slice(&(len as u16).to_be_bytes());
            assert!(parse_disco_frame(&bytes).is_none());
        }
        let mut extra = canonical.clone();
        extra.push(0);
        assert!(parse_disco_frame(&extra).is_none());
        for payload_len in [0, 12, 257] {
            let mut header = [0xa5; FRAME_HEADER_SIZE];
            assert_eq!(
                build_disco_frame(
                    &mut header,
                    BROADCAST_MAC,
                    [1; 6],
                    FLAG_PROBE,
                    &[2; 16],
                    payload_len
                ),
                None
            );
            assert_eq!(header, [0xa5; FRAME_HEADER_SIZE]);
        }
        let mut header = [0xa5; FRAME_HEADER_SIZE];
        assert_eq!(
            build_disco_frame(
                &mut header,
                BROADCAST_MAC,
                [1; 6],
                FLAG_PROBE | FLAG_RESPONSE,
                &[2; 16],
                13
            ),
            None
        );
        assert_eq!(header, [0xa5; FRAME_HEADER_SIZE]);
    }

    #[test]
    fn ethernet_padding_cannot_resurrect_a_retired_base_only_frame() {
        let no_cluster = PeerAdvertisement {
            role: ROLE_NO_CLUSTER,
            raft_id: b"",
            leader_id: b"",
            ..peer()
        };
        for flags in [FLAG_PROBE, FLAG_RESPONSE] {
            let bytes = frame(flags, no_cluster);
            assert!(bytes.len() < MIN_ETHERNET_FRAME_SIZE);
            assert_eq!(parse_disco_frame(&bytes).unwrap().peer, no_cluster);
            let mut padded = bytes.clone();
            padded.resize(MIN_ETHERNET_FRAME_SIZE, 0);
            assert_eq!(parse_disco_frame(&padded).unwrap().peer, no_cluster);
            let mut nonzero = padded.clone();
            *nonzero.last_mut().unwrap() = 1;
            assert!(parse_disco_frame(&nonzero).is_none());
            assert!(parse_disco_frame(&padded[..MIN_ETHERNET_FRAME_SIZE - 1]).is_none());
            padded.push(0);
            assert!(parse_disco_frame(&padded).is_none());
            // Old headers did not declare a payload length. Padding alone
            // could otherwise look like a complete empty cluster block.
            let mut retired = bytes.clone();
            retired.truncate(FRAME_HEADER_SIZE + 1 + no_cluster.node_id.len() + 8);
            retired[24..26].fill(0);
            assert!(parse_disco_frame(&retired).is_none());
            retired.resize(MIN_ETHERNET_FRAME_SIZE, 0);
            assert!(parse_disco_frame(&retired).is_none());
        }
    }

    #[test]
    fn cluster_answer_rejects_short_buffers_without_panicking() {
        for len in 0..10 {
            let mut buf = alloc::vec![0u8; len];
            assert_eq!(build_cluster_answer(&mut buf, ROLE_LEADER, b"n1", b"n1", &[]), None);
        }
    }

    #[test]
    fn cluster_answer_round_trips() {
        let peers = [PeerClusterInfo {
            mac: [1, 2, 3, 4, 5, 6],
            role: ROLE_LEADER,
            raft_id: b"n2",
            leader_id: b"n2",
        }];
        let mut buf = [0u8; 64];
        let len = build_cluster_answer(&mut buf, ROLE_LEADER, b"n1", b"n1", &peers).unwrap();
        let parsed = parse_cluster_answer(&buf[..len]).unwrap();
        assert_eq!(parsed.0, ROLE_LEADER);
        assert_eq!(parsed.1, b"n1");
        assert_eq!(parsed.2, b"n1");
        assert_eq!(parsed.3[0].0, peers[0].mac);
        assert_eq!(parsed.3[0].2, b"n2");
    }
}
