//! Pure policy and packet helpers for distributed L2 ingress.
//!
//! The frame router owns the mutable integration state. This module keeps the
//! service identity, committed backend snapshot, five-tuple extraction and
//! rendezvous selection independent from IPC, Raft and packet transmission so
//! the hot-path decision is deterministic and directly testable.

use alloc::{
    vec,
    vec::Vec,
};

pub const IPV4_ETHERTYPE: u16 = 0x0800;
pub const ARP_ETHERTYPE: u16 = 0x0806;
/// Charlotte-internal L2 forwarding envelope. Frames with this EtherType must
/// only be accepted from a MAC in the current committed member snapshot.
pub const FORWARDED_ETHERTYPE: u16 = 0x88b8;
pub const FORWARD_ENVELOPE_LEN: usize = 8;
pub const IP_PROTOCOL_TCP: u8 = 6;
pub const MAX_BACKENDS: usize = 64;
pub const MEMBERSHIP_MAGIC: u32 = 0x314d_424c; // "LBM1"
pub const MEMBERSHIP_VERSION: u16 = 2;
pub const MEMBERSHIP_HEADER_LEN: usize = 40;
pub const MEMBERSHIP_RECORD_LEN: usize = 16;
const MEMBER_FLAG_ELIGIBLE: u8 = 1 << 0;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ServiceId {
    pub address: [u8; 4],
    pub protocol: u8,
    pub port: u16,
}

impl ServiceId {
    pub const fn tcp_v4(address: [u8; 4], port: u16) -> Self {
        Self {
            address,
            protocol: IP_PROTOCOL_TCP,
            port,
        }
    }

    pub fn is_valid(self) -> bool {
        self.address != [0; 4] && self.port != 0 && self.protocol == IP_PROTOCOL_TCP
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FlowKey {
    pub protocol: u8,
    pub src_addr: [u8; 4],
    pub dst_addr: [u8; 4],
    pub src_port: u16,
    pub dst_port: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedFlow {
    pub key: FlowKey,
    pub initial_syn: bool,
    pub terminal: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Backend {
    /// Stable node token derived from the committed Charlotte identity.
    pub node_id: u64,
    /// Current discovery-authenticated Ethernet route for that identity.
    pub mac: [u8; 6],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendSnapshot {
    /// Deterministic fingerprint of the committed Raft configuration and
    /// replicated ingress-drain policy represented here.
    pub epoch: u64,
    pub self_node: u64,
    advertiser_node: Option<u64>,
    /// Every admitted member with a complete discovery route. This larger
    /// set remains authoritative for accepting one-hop forwarding envelopes
    /// and retaining established flows while a member drains.
    members: Vec<Backend>,
    /// Members accepting new flows. This is a subset of `members`.
    backends: Vec<Backend>,
}

impl BackendSnapshot {
    pub fn new(epoch: u64, self_node: u64, mut backends: Vec<Backend>) -> Option<Self> {
        backends.sort_unstable_by_key(|backend| backend.node_id);
        let advertiser_node = backends.first().map(|backend| backend.node_id);
        Self::new_with_advertiser(epoch, self_node, advertiser_node, backends)
    }

    pub fn new_with_advertiser(
        epoch: u64,
        self_node: u64,
        advertiser_node: Option<u64>,
        backends: Vec<Backend>,
    ) -> Option<Self> {
        let eligible = backends.iter().map(|backend| backend.node_id).collect();
        Self::new_with_members(epoch, self_node, advertiser_node, backends, eligible)
    }

    pub fn new_with_members(
        epoch: u64,
        self_node: u64,
        advertiser_node: Option<u64>,
        mut members: Vec<Backend>,
        mut eligible_nodes: Vec<u64>,
    ) -> Option<Self> {
        if members.is_empty()
            || members.len() > MAX_BACKENDS
            || eligible_nodes.len() > members.len()
        {
            return None;
        }
        members.sort_unstable_by_key(|backend| backend.node_id);
        eligible_nodes.sort_unstable();
        if members.iter().any(|backend| backend.mac == [0; 6] || backend.mac[0] & 1 != 0)
            || members.iter().enumerate().any(|(index, backend)| {
                members[index + 1..]
                    .iter()
                    .any(|other| backend.node_id == other.node_id || backend.mac == other.mac)
            })
            || eligible_nodes.windows(2).any(|pair| pair[0] == pair[1])
            || eligible_nodes
                .iter()
                .any(|node_id| !members.iter().any(|member| member.node_id == *node_id))
            || advertiser_node.is_some_and(|advertiser| !eligible_nodes.contains(&advertiser))
        {
            return None;
        }
        let backends = members
            .iter()
            .filter(|member| eligible_nodes.binary_search(&member.node_id).is_ok())
            .copied()
            .collect();
        Some(Self {
            epoch,
            self_node,
            advertiser_node,
            members,
            backends,
        })
    }

    pub fn members(&self) -> &[Backend] {
        &self.members
    }

    pub fn backends(&self) -> &[Backend] {
        &self.backends
    }

    pub fn vip_advertiser(&self) -> Option<Backend> {
        let advertiser = self.advertiser_node?;
        self.backends.iter().find(|backend| backend.node_id == advertiser).copied()
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes =
            vec![0u8; MEMBERSHIP_HEADER_LEN + self.members.len() * MEMBERSHIP_RECORD_LEN];
        bytes[0..4].copy_from_slice(&MEMBERSHIP_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&MEMBERSHIP_VERSION.to_le_bytes());
        bytes[6..8].copy_from_slice(&(MEMBERSHIP_HEADER_LEN as u16).to_le_bytes());
        bytes[8..16].copy_from_slice(&self.epoch.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.self_node.to_le_bytes());
        bytes[24..32].copy_from_slice(&self.advertiser_node.unwrap_or(u64::MAX).to_le_bytes());
        bytes[32..34].copy_from_slice(&(self.members.len() as u16).to_le_bytes());
        for (index, backend) in self.members.iter().enumerate() {
            let offset = MEMBERSHIP_HEADER_LEN + index * MEMBERSHIP_RECORD_LEN;
            bytes[offset..offset + 8].copy_from_slice(&backend.node_id.to_le_bytes());
            bytes[offset + 8..offset + 14].copy_from_slice(&backend.mac);
            if self.backends.binary_search_by_key(&backend.node_id, |item| item.node_id).is_ok() {
                bytes[offset + 14] = MEMBER_FLAG_ELIGIBLE;
            }
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < MEMBERSHIP_HEADER_LEN
            || u32::from_le_bytes(bytes[0..4].try_into().ok()?) != MEMBERSHIP_MAGIC
            || !matches!(u16::from_le_bytes(bytes[4..6].try_into().ok()?), 1 | MEMBERSHIP_VERSION)
            || usize::from(u16::from_le_bytes(bytes[6..8].try_into().ok()?))
                != MEMBERSHIP_HEADER_LEN
            || bytes[34..40].iter().any(|byte| *byte != 0)
        {
            return None;
        }
        let version = u16::from_le_bytes(bytes[4..6].try_into().ok()?);
        let count = usize::from(u16::from_le_bytes(bytes[32..34].try_into().ok()?));
        let expected =
            MEMBERSHIP_HEADER_LEN.checked_add(count.checked_mul(MEMBERSHIP_RECORD_LEN)?)?;
        if bytes.len() != expected || count == 0 || count > MAX_BACKENDS {
            return None;
        }
        let mut members = Vec::with_capacity(count);
        let mut eligible_nodes = Vec::with_capacity(count);
        for index in 0..count {
            let offset = MEMBERSHIP_HEADER_LEN + index * MEMBERSHIP_RECORD_LEN;
            let flags = bytes[offset + 14];
            if bytes[offset + 15] != 0
                || (version == 1 && flags != 0)
                || flags & !MEMBER_FLAG_ELIGIBLE != 0
            {
                return None;
            }
            let backend = Backend {
                node_id: u64::from_le_bytes(bytes[offset..offset + 8].try_into().ok()?),
                mac: bytes[offset + 8..offset + 14].try_into().ok()?,
            };
            if version == 1 || flags & MEMBER_FLAG_ELIGIBLE != 0 {
                eligible_nodes.push(backend.node_id);
            }
            members.push(backend);
        }
        let advertiser = u64::from_le_bytes(bytes[24..32].try_into().ok()?);
        Self::new_with_members(
            u64::from_le_bytes(bytes[8..16].try_into().ok()?),
            u64::from_le_bytes(bytes[16..24].try_into().ok()?),
            (advertiser != u64::MAX).then_some(advertiser),
            members,
            eligible_nodes,
        )
    }
}

/// Produce the load-balancing epoch from committed membership plus replicated
/// drain generations. The explicit hash is stable on every node and changes
/// only when the backend policy changes, not for unrelated catalog traffic.
pub fn load_balancing_epoch(membership_epoch: u64, draining_nodes: &[(u64, u64)]) -> u64 {
    let mut draining_nodes = draining_nodes.to_vec();
    draining_nodes.sort_unstable();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    hash_bytes(&mut hash, b"Charlotte ingress policy v1\0");
    hash_bytes(&mut hash, &membership_epoch.to_le_bytes());
    for (node_id, generation) in draining_nodes {
        hash_bytes(&mut hash, &node_id.to_le_bytes());
        hash_bytes(&mut hash, &generation.to_le_bytes());
    }
    avalanche(hash)
}

/// Select one backend using highest-random-weight (rendezvous) hashing.
///
/// `snapshot.epoch` deliberately does not seed the score. The epoch selects
/// the backend *set* retained for a flow; keeping it out of the hash means an
/// addition only moves flows won by the new member instead of reshuffling all
/// connections.
pub fn select_backend(
    service: &ServiceId,
    flow: &FlowKey,
    snapshot: &BackendSnapshot,
) -> Option<Backend> {
    if !service.is_valid() || flow.protocol != service.protocol {
        return None;
    }
    snapshot.backends.iter().copied().max_by(|left, right| {
        rendezvous_score(service, flow, left.node_id)
            .cmp(&rendezvous_score(service, flow, right.node_id))
            .then_with(|| left.node_id.cmp(&right.node_id))
    })
}

fn rendezvous_score(service: &ServiceId, flow: &FlowKey, node_id: u64) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    hash_bytes(&mut hash, b"Charlotte L2 rendezvous v1\0");
    hash_bytes(&mut hash, &service.address);
    hash_bytes(&mut hash, &[service.protocol]);
    hash_bytes(&mut hash, &service.port.to_be_bytes());
    hash_bytes(&mut hash, &[flow.protocol]);
    hash_bytes(&mut hash, &flow.src_addr);
    hash_bytes(&mut hash, &flow.dst_addr);
    hash_bytes(&mut hash, &flow.src_port.to_be_bytes());
    hash_bytes(&mut hash, &flow.dst_port.to_be_bytes());
    hash_bytes(&mut hash, &node_id.to_le_bytes());
    avalanche(hash)
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn avalanche(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FlowBinding {
    flow: FlowKey,
    epoch: u64,
    last_seen: u64,
}

/// Bounded ingress-local flow-to-membership-epoch cache.
pub struct FlowEpochTable {
    entries: Vec<FlowBinding>,
    capacity: usize,
    clock: u64,
}

impl FlowEpochTable {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            capacity,
            clock: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return the epoch to use for this packet. Existing entries win even for
    /// a retransmitted initial SYN. FIN/RST releases the entry after returning
    /// its pinned epoch for the terminal packet itself.
    pub fn observe(&mut self, packet: &ParsedFlow, current_epoch: u64) -> u64 {
        self.clock = self.clock.wrapping_add(1).max(1);
        if let Some(index) = self.entries.iter().position(|entry| entry.flow == packet.key) {
            let epoch = self.entries[index].epoch;
            if packet.terminal {
                self.entries.swap_remove(index);
            } else {
                self.entries[index].last_seen = self.clock;
            }
            return epoch;
        }
        if !packet.terminal && self.capacity != 0 {
            if self.entries.len() == self.capacity {
                let oldest = self
                    .entries
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, entry)| entry.last_seen)
                    .map(|(index, _)| index)
                    .expect("full flow table has an oldest entry");
                self.entries.swap_remove(oldest);
            }
            self.entries.push(FlowBinding {
                flow: packet.key,
                epoch: current_epoch,
                last_seen: self.clock,
            });
        }
        current_epoch
    }

    /// Forget only flows whose previously selected backend is absent from a
    /// new committed set. Flows owned by surviving nodes retain their old
    /// epoch, while a reconnect formerly owned by a removed node can select
    /// an active backend on its next SYN.
    pub fn remove_absent_backends(
        &mut self,
        service: &ServiceId,
        history: &SnapshotHistory,
        current: &BackendSnapshot,
    ) -> usize {
        let before = self.entries.len();
        self.entries.retain(|entry| {
            history
                .get(entry.epoch)
                .and_then(|snapshot| select_backend(service, &entry.flow, snapshot))
                .is_some_and(|selected| {
                    current.members.iter().any(|backend| backend.node_id == selected.node_id)
                })
        });
        before - self.entries.len()
    }
}

/// A bounded set of recent immutable backend snapshots.
pub struct SnapshotHistory {
    snapshots: Vec<BackendSnapshot>,
    capacity: usize,
}

impl SnapshotHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            snapshots: Vec::with_capacity(capacity),
            capacity,
        }
    }

    pub fn install(&mut self, snapshot: BackendSnapshot) {
        if self.capacity == 0 {
            return;
        }
        if let Some(index) = self.snapshots.iter().position(|item| item.epoch == snapshot.epoch) {
            self.snapshots[index] = snapshot;
            return;
        }
        if self.snapshots.len() == self.capacity {
            self.snapshots.remove(0);
        }
        self.snapshots.push(snapshot);
    }

    pub fn current(&self) -> Option<&BackendSnapshot> {
        self.snapshots.last()
    }

    pub fn get(&self, epoch: u64) -> Option<&BackendSnapshot> {
        self.snapshots.iter().find(|snapshot| snapshot.epoch == epoch)
    }
}

/// Parse an Ethernet/IPv4/TCP frame only when it targets `service`.
pub fn parse_service_flow(frame: &[u8], service: &ServiceId) -> Option<ParsedFlow> {
    if !service.is_valid()
        || frame.len() < 14 + 20 + 20
        || u16::from_be_bytes(frame[12..14].try_into().ok()?) != IPV4_ETHERTYPE
    {
        return None;
    }
    let ip = &frame[14..];
    if ip[0] >> 4 != 4 || ip[9] != service.protocol {
        return None;
    }
    let ip_header_len = usize::from(ip[0] & 0x0f).checked_mul(4)?;
    if ip_header_len < 20 || ip.len() < ip_header_len + 20 {
        return None;
    }
    let total_len = usize::from(u16::from_be_bytes(ip[2..4].try_into().ok()?));
    if total_len < ip_header_len + 20 || total_len > ip.len() {
        return None;
    }
    // Non-initial fragments do not contain a stable TCP header. Initial
    // fragmented SYNs are also excluded so every participant makes the same
    // fail-closed decision without maintaining an IP reassembly side table.
    if u16::from_be_bytes(ip[6..8].try_into().ok()?) & 0x3fff != 0 {
        return None;
    }
    let dst_addr: [u8; 4] = ip[16..20].try_into().ok()?;
    if dst_addr != service.address {
        return None;
    }
    let tcp = &ip[ip_header_len..total_len];
    let src_port = u16::from_be_bytes(tcp[0..2].try_into().ok()?);
    let dst_port = u16::from_be_bytes(tcp[2..4].try_into().ok()?);
    let tcp_header_len = usize::from(tcp[12] >> 4) * 4;
    if dst_port != service.port || tcp_header_len < 20 || tcp_header_len > tcp.len() {
        return None;
    }
    let flags = tcp[13];
    Some(ParsedFlow {
        key: FlowKey {
            protocol: service.protocol,
            src_addr: ip[12..16].try_into().ok()?,
            dst_addr,
            src_port,
            dst_port,
        },
        initial_syn: flags & 0x02 != 0 && flags & 0x10 == 0,
        terminal: flags & 0x05 != 0,
    })
}

pub fn is_arp_request_for_vip(frame: &[u8], vip: [u8; 4]) -> bool {
    frame.len() >= 42
        && u16::from_be_bytes([frame[12], frame[13]]) == ARP_ETHERTYPE
        && u16::from_be_bytes([frame[14], frame[15]]) == 1
        && u16::from_be_bytes([frame[16], frame[17]]) == IPV4_ETHERTYPE
        && frame[18] == 6
        && frame[19] == 4
        && u16::from_be_bytes([frame[20], frame[21]]) == 1
        && frame[38..42] == vip
}

/// Whether this participant may advertise the VIP represented by `snapshot`.
/// Absence of a complete committed snapshot fails closed.
pub fn local_advertises_vip(snapshot: Option<&BackendSnapshot>) -> bool {
    snapshot.is_some_and(|snapshot| {
        snapshot.vip_advertiser().is_some_and(|owner| owner.node_id == snapshot.self_node)
    })
}

/// Wrap an ordinary Ethernet frame for one-hop delivery to a selected
/// Charlotte backend. The original source MAC and EtherType are carried in a
/// compact in-frame envelope so the backend can restore the packet before
/// passing it to its local protocol stack. The private EtherType also prevents
/// a receiving frouter from rendezvous-selecting an already forwarded packet
/// a second time while membership epochs converge.
pub fn encapsulate_forwarded_frame(
    frame: &mut [u8],
    frame_len: usize,
    ingress_mac: [u8; 6],
    destination: [u8; 6],
) -> Option<usize> {
    if frame_len < 14 || frame_len.checked_add(FORWARD_ENVELOPE_LEN)? > frame.len() {
        return None;
    }
    let original_source: [u8; 6] = frame[6..12].try_into().ok()?;
    let original_ethertype: [u8; 2] = frame[12..14].try_into().ok()?;
    frame.copy_within(14..frame_len, 14 + FORWARD_ENVELOPE_LEN);
    frame[0..6].copy_from_slice(&destination);
    frame[6..12].copy_from_slice(&ingress_mac);
    frame[12..14].copy_from_slice(&FORWARDED_ETHERTYPE.to_be_bytes());
    frame[14..20].copy_from_slice(&original_source);
    frame[20..22].copy_from_slice(&original_ethertype);
    Some(frame_len + FORWARD_ENVELOPE_LEN)
}

/// Remove a trusted one-hop Charlotte forwarding envelope and restore the
/// Ethernet source and EtherType seen at the ingress participant. The outer
/// destination remains the selected backend's local MAC.
pub fn decapsulate_forwarded_frame(frame: &mut [u8], frame_len: usize) -> Option<(usize, u16)> {
    if frame_len < 14 + FORWARD_ENVELOPE_LEN
        || u16::from_be_bytes(frame[12..14].try_into().ok()?) != FORWARDED_ETHERTYPE
    {
        return None;
    }
    let original_source: [u8; 6] = frame[14..20].try_into().ok()?;
    let original_ethertype = u16::from_be_bytes(frame[20..22].try_into().ok()?);
    frame.copy_within(14 + FORWARD_ENVELOPE_LEN..frame_len, 14);
    frame[6..12].copy_from_slice(&original_source);
    frame[12..14].copy_from_slice(&original_ethertype.to_be_bytes());
    Some((frame_len - FORWARD_ENVELOPE_LEN, original_ethertype))
}

/// Construct an Ethernet/IPv4 gratuitous ARP reply for a newly selected VIP
/// advertiser. The frame has no internal Charlotte metadata and remains safe
/// for ordinary switches and hosts to consume.
pub fn gratuitous_arp(mac: [u8; 6], vip: [u8; 4]) -> [u8; 42] {
    let mut frame = [0u8; 42];
    frame[0..6].fill(0xff);
    frame[6..12].copy_from_slice(&mac);
    frame[12..14].copy_from_slice(&ARP_ETHERTYPE.to_be_bytes());
    frame[14..16].copy_from_slice(&1u16.to_be_bytes());
    frame[16..18].copy_from_slice(&IPV4_ETHERTYPE.to_be_bytes());
    frame[18] = 6;
    frame[19] = 4;
    frame[20..22].copy_from_slice(&2u16.to_be_bytes());
    frame[22..28].copy_from_slice(&mac);
    frame[28..32].copy_from_slice(&vip);
    frame[32..38].fill(0xff);
    frame[38..42].copy_from_slice(&vip);
    frame
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    const SERVICE: ServiceId = ServiceId::tcp_v4([10, 0, 0, 42], 443);

    fn backend(id: u64) -> Backend {
        Backend {
            node_id: id,
            mac: [0x02, 0, 0, 0, 0, id as u8],
        }
    }

    fn snapshot(epoch: u64, ids: &[u64]) -> BackendSnapshot {
        BackendSnapshot::new(epoch, ids[0], ids.iter().copied().map(backend).collect()).unwrap()
    }

    fn flow(index: u32) -> FlowKey {
        FlowKey {
            protocol: IP_PROTOCOL_TCP,
            src_addr: [192, 0, (index >> 8) as u8, index as u8],
            dst_addr: SERVICE.address,
            src_port: 1024 + (index % 60_000) as u16,
            dst_port: SERVICE.port,
        }
    }

    fn frame(key: FlowKey, flags: u8) -> Vec<u8> {
        let mut bytes = vec![0u8; 14 + 20 + 20 + 7];
        let ip_len = (bytes.len() - 14) as u16;
        bytes[0..6].copy_from_slice(&[0x02, 0, 0, 0, 0, 1]);
        bytes[6..12].copy_from_slice(&[0x02, 0, 0, 0, 0, 99]);
        bytes[12..14].copy_from_slice(&IPV4_ETHERTYPE.to_be_bytes());
        bytes[14] = 0x45;
        bytes[16..18].copy_from_slice(&ip_len.to_be_bytes());
        bytes[23] = IP_PROTOCOL_TCP;
        bytes[26..30].copy_from_slice(&key.src_addr);
        bytes[30..34].copy_from_slice(&key.dst_addr);
        bytes[34..36].copy_from_slice(&key.src_port.to_be_bytes());
        bytes[36..38].copy_from_slice(&key.dst_port.to_be_bytes());
        bytes[46] = 0x50;
        bytes[47] = flags;
        bytes[54..].copy_from_slice(b"payload");
        bytes
    }

    #[test]
    fn selection_is_order_independent_and_snapshot_round_trips() {
        let one = snapshot(17, &[1, 2, 3]);
        let two = BackendSnapshot::new(17, 1, vec![backend(3), backend(1), backend(2)]).unwrap();
        assert_eq!(one, BackendSnapshot::decode(&one.encode()).unwrap());
        for index in 0..1_000 {
            assert_eq!(
                select_backend(&SERVICE, &flow(index), &one),
                select_backend(&SERVICE, &flow(index), &two)
            );
        }
        let mut history = SnapshotHistory::new(2);
        history.install(snapshot(18, &[1, 2, 3]));
        history.install(snapshot(17, &[1, 2, 3]));
        // Policy epochs are opaque fingerprints, so arrival order rather
        // than numeric comparison determines the current local snapshot.
        assert_eq!(history.current().unwrap().epoch, 17);
    }

    #[test]
    fn policy_epoch_is_deterministic_and_changes_with_drain_state() {
        let one = load_balancing_epoch(17, &[(3, 1), (2, 4)]);
        let reordered = load_balancing_epoch(17, &[(2, 4), (3, 1)]);
        assert_eq!(one, reordered);
        assert_ne!(one, load_balancing_epoch(18, &[(2, 4), (3, 1)]));
        assert_ne!(one, load_balancing_epoch(17, &[(2, 5), (3, 1)]));
    }

    #[test]
    fn rendezvous_distribution_is_balanced() {
        let members = snapshot(17, &[1, 2, 3]);
        let mut counts = [0usize; 3];
        for index in 0..30_000 {
            let selected = select_backend(&SERVICE, &flow(index), &members).unwrap();
            counts[selected.node_id as usize - 1] += 1;
        }
        for count in counts {
            assert!((8_500..=11_500).contains(&count), "unbalanced count {count}");
        }
    }

    #[test]
    fn adding_a_member_only_moves_its_winners() {
        let before = snapshot(17, &[1, 2, 3]);
        let after = snapshot(18, &[1, 2, 3, 4]);
        let mut moved = 0usize;
        let mut selected_four = 0usize;
        for index in 0..20_000 {
            let old = select_backend(&SERVICE, &flow(index), &before).unwrap();
            let new = select_backend(&SERVICE, &flow(index), &after).unwrap();
            if old != new {
                moved += 1;
                assert_eq!(new.node_id, 4);
            }
            selected_four += usize::from(new.node_id == 4);
        }
        assert!((3_500..=6_500).contains(&moved));
        assert_eq!(moved, selected_four);
    }

    #[test]
    fn removal_preserves_survivors_and_releases_failed_backend_bindings() {
        let before = snapshot(17, &[1, 2, 3]);
        let after = snapshot(18, &[1, 3]);
        let mut history = SnapshotHistory::new(4);
        history.install(before.clone());
        history.install(after.clone());
        let mut table = FlowEpochTable::new(20_000);
        let mut removed = 0usize;
        let mut surviving = 0usize;
        for index in 0..10_000 {
            let key = flow(index);
            let packet = ParsedFlow {
                key,
                initial_syn: true,
                terminal: false,
            };
            table.observe(&packet, before.epoch);
            match select_backend(&SERVICE, &key, &before).unwrap().node_id {
                2 => removed += 1,
                _ => surviving += 1,
            }
        }
        assert_eq!(table.remove_absent_backends(&SERVICE, &history, &after), removed);
        assert_eq!(table.len(), surviving);
    }

    #[test]
    fn drain_stops_new_selection_but_retains_existing_backend_binding() {
        let before = snapshot(17, &[1, 2, 3]);
        let after = BackendSnapshot::new_with_members(
            18,
            1,
            Some(1),
            vec![backend(1), backend(2), backend(3)],
            vec![1, 3],
        )
        .unwrap();
        assert_eq!(after.members().len(), 3);
        assert_eq!(after.backends().iter().map(|item| item.node_id).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(after, BackendSnapshot::decode(&after.encode()).unwrap());

        let (index, key) = (0..10_000)
            .map(|index| (index, flow(index)))
            .find(|(_, key)| select_backend(&SERVICE, key, &before).unwrap().node_id == 2)
            .expect("one test flow selects the draining backend");
        let mut history = SnapshotHistory::new(4);
        history.install(before.clone());
        history.install(after.clone());
        let mut table = FlowEpochTable::new(1);
        table.observe(
            &ParsedFlow {
                key,
                initial_syn: true,
                terminal: false,
            },
            before.epoch,
        );
        assert_eq!(table.remove_absent_backends(&SERVICE, &history, &after), 0);
        let retained = history.get(table.observe(
            &ParsedFlow {
                key,
                initial_syn: false,
                terminal: false,
            },
            after.epoch,
        ));
        assert_eq!(select_backend(&SERVICE, &key, retained.unwrap()).unwrap().node_id, 2);
        assert_ne!(select_backend(&SERVICE, &flow(index + 1), &after).unwrap().node_id, 2);
    }

    #[test]
    fn draining_every_member_produces_a_complete_fail_closed_snapshot() {
        let drained =
            BackendSnapshot::new_with_members(19, 1, None, vec![backend(1), backend(2)], vec![])
                .unwrap();
        assert_eq!(drained.members().len(), 2);
        assert!(drained.backends().is_empty());
        assert_eq!(drained.vip_advertiser(), None);
        assert_eq!(select_backend(&SERVICE, &flow(1), &drained), None);
        assert_eq!(drained, BackendSnapshot::decode(&drained.encode()).unwrap());
    }

    #[test]
    fn observed_flow_keeps_its_epoch_across_membership_change() {
        let key = flow(9);
        let syn = ParsedFlow {
            key,
            initial_syn: true,
            terminal: false,
        };
        let data = ParsedFlow {
            key,
            initial_syn: false,
            terminal: false,
        };
        let mut table = FlowEpochTable::new(16);
        assert_eq!(table.observe(&syn, 17), 17);
        assert_eq!(table.observe(&syn, 18), 17);
        assert_eq!(table.observe(&data, 18), 17);
    }

    #[test]
    fn replacement_ingress_selects_same_backend_without_shared_state() {
        let members = snapshot(17, &[1, 2, 3]);
        let key = flow(42);
        let from_a = select_backend(&SERVICE, &key, &members).unwrap();
        let replacement =
            BackendSnapshot::new_with_advertiser(17, 3, Some(3), members.backends().to_vec())
                .unwrap();
        let independently_decoded = BackendSnapshot::decode(&replacement.encode()).unwrap();
        let from_c = select_backend(&SERVICE, &key, &independently_decoded).unwrap();
        assert_eq!(from_a, from_c);
        assert!(local_advertises_vip(Some(&independently_decoded)));
    }

    #[test]
    fn forwarding_envelope_round_trip_preserves_the_ip_packet() {
        let key = flow(1);
        let mut packet = frame(key, 0x02);
        packet.resize(packet.len() + FORWARD_ENVELOPE_LEN, 0);
        let parsed = parse_service_flow(&packet, &SERVICE).unwrap();
        assert_eq!(parsed.key, key);
        assert!(parsed.initial_syn);
        let original = packet[..packet.len() - FORWARD_ENVELOPE_LEN].to_vec();
        let ingress = backend(1).mac;
        let forwarded_len =
            encapsulate_forwarded_frame(&mut packet, original.len(), ingress, backend(2).mac)
                .unwrap();
        assert_eq!(forwarded_len, original.len() + FORWARD_ENVELOPE_LEN);
        assert_eq!(&packet[0..6], &backend(2).mac);
        assert_eq!(&packet[6..12], &ingress);
        assert_eq!(u16::from_be_bytes(packet[12..14].try_into().unwrap()), FORWARDED_ETHERTYPE);

        let (restored_len, restored_ethertype) =
            decapsulate_forwarded_frame(&mut packet, forwarded_len).unwrap();
        assert_eq!(restored_len, original.len());
        assert_eq!(restored_ethertype, IPV4_ETHERTYPE);
        assert_eq!(&packet[..6], &backend(2).mac);
        assert_eq!(&packet[6..restored_len], &original[6..]);
        assert_eq!(parse_service_flow(&packet[..restored_len], &SERVICE).unwrap().key, key);
    }

    #[test]
    fn forwarding_envelope_is_bounded_and_type_checked() {
        let mut packet = frame(flow(1), 0x02);
        let exact_len = packet.len();
        assert!(
            encapsulate_forwarded_frame(&mut packet, exact_len, backend(1).mac, backend(2).mac)
                .is_none()
        );
        packet.resize(exact_len + FORWARD_ENVELOPE_LEN, 0);
        assert!(decapsulate_forwarded_frame(&mut packet, exact_len).is_none());
    }

    #[test]
    fn arp_matching_and_gratuitous_advertisement_are_vip_specific() {
        let mut request = [0u8; 42];
        request[12..14].copy_from_slice(&ARP_ETHERTYPE.to_be_bytes());
        request[14..16].copy_from_slice(&1u16.to_be_bytes());
        request[16..18].copy_from_slice(&IPV4_ETHERTYPE.to_be_bytes());
        request[18] = 6;
        request[19] = 4;
        request[20..22].copy_from_slice(&1u16.to_be_bytes());
        request[38..42].copy_from_slice(&SERVICE.address);
        assert!(is_arp_request_for_vip(&request, SERVICE.address));
        assert!(!is_arp_request_for_vip(&request, [10, 0, 0, 43]));

        let owner = snapshot(17, &[1, 2, 3]);
        let non_owner = BackendSnapshot::new(17, 2, owner.backends().to_vec()).unwrap();
        assert!(local_advertises_vip(Some(&owner)));
        assert!(!local_advertises_vip(Some(&non_owner)));
        assert!(!local_advertises_vip(None));

        let announcement = gratuitous_arp(backend(1).mac, SERVICE.address);
        assert_eq!(&announcement[0..6], &[0xff; 6]);
        assert_eq!(&announcement[28..32], &SERVICE.address);
        assert_eq!(&announcement[38..42], &SERVICE.address);
    }
}
