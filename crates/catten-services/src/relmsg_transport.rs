//! Raft peer transport over the reliable message layer (`relmsg`).
//!
//! Implements `catten_graft::RaftTransport` by carrying Vote/AppendEntries/
//! InstallSnapshot RPCs as `relmsg` messages addressed to peer node MACs,
//! routed through the frame demultiplexer to the NIC. Each message is prefixed
//! with a one-byte request/response type tag followed by the encoded protobuf.
//!
//! - Requests are handed to the owning reactor (which drives `RaftNode::handle_*`) and answered
//!   with a tagged response back to the source MAC.
//! - Responses are buffered and surfaced through `poll_completions`, exactly like
//!   `CharlotteTransport`.
//!
//! relmsg allows one outstanding `OP_SEND` per peer (it replies `ERR_BUSY`
//! otherwise), so outbound RPCs are queued per peer and drained one at a time.
use alloc::{
    collections::BTreeMap,
    string::{
        String,
        ToString,
    },
    vec::Vec,
};

use catten_graft::{
    transport::{
        AppendEntriesRpc,
        InstallSnapshotRpc,
        RaftTransport,
        RpcCompletion,
    },
    types::Peer,
    wire::{
        decode_append_request,
        decode_append_response,
        decode_snapshot_request,
        decode_snapshot_response,
        decode_vote_request,
        decode_vote_response,
        encode_append_request,
        encode_snapshot_request,
        encode_vote_request,
    },
};
use catten_syscall::*;

const SCRATCH: usize = 0x0000_0000_0083_0000;

pub const TAG_VOTE_REQUEST: u8 = 1;
pub const TAG_APPEND_REQUEST: u8 = 2;
pub const TAG_SNAPSHOT_REQUEST: u8 = 3;
pub const TAG_VOTE_RESPONSE: u8 = 4;
pub const TAG_APPEND_RESPONSE: u8 = 5;
pub const TAG_SNAPSHOT_RESPONSE: u8 = 6;
/// Pre-membership join handshake: a node that has located a cluster asks the
/// leader to admit it. These are *network* frames addressed by MAC — before
/// admission the joiner has no local name-service route to any raft peer.
pub const TAG_JOIN_REQUEST: u8 = 7;
pub const TAG_JOIN_REPLY: u8 = 8;

/// Encode the body of a join request: `[id_len][id][service_name:8 LE]`.
///
/// [`RelmsgRaftTransport::send_message`] owns the outer message tag, just as
/// it does for the protobuf Raft RPC payloads. Keeping the tag out of this
/// body avoids producing `[tag][tag][body]` on the wire.
pub fn encode_join_request(joiner_id: &[u8], service_name: u64) -> Option<alloc::vec::Vec<u8>> {
    if joiner_id.is_empty() || joiner_id.len() > 255 {
        return None;
    }
    let total = 1 + joiner_id.len() + 8;
    if total > MAX_RPC_PAYLOAD {
        return None;
    }
    let mut frame = alloc::vec::Vec::with_capacity(total);
    frame.push(joiner_id.len() as u8);
    frame.extend_from_slice(joiner_id);
    frame.extend_from_slice(&service_name.to_le_bytes());
    Some(frame)
}

/// Decode a join-request body into `(joiner_id, joiner_raft_service_name)`.
pub fn decode_join_request(frame: &[u8]) -> Option<(&[u8], u64)> {
    if frame.len() < 2 {
        return None;
    }
    let id_len = frame[0] as usize;
    if id_len == 0 || frame.len() != 1 + id_len + 8 {
        return None;
    }
    let id = &frame[1..1 + id_len];
    let service_name = u64::from_le_bytes(frame[1 + id_len..1 + id_len + 8].try_into().ok()?);
    Some((id, service_name))
}

/// Encode the body of a join reply carrying the committed JOIN log index
/// (0 = refused). The transport prepends [`TAG_JOIN_REPLY`].
pub fn encode_join_reply(join_index: u64) -> alloc::vec::Vec<u8> {
    let mut frame = alloc::vec::Vec::with_capacity(8);
    frame.extend_from_slice(&join_index.to_le_bytes());
    frame
}

/// Decode a join-reply body into the committed JOIN log index.
pub fn decode_join_reply(frame: &[u8]) -> Option<u64> {
    if frame.len() != 8 {
        return None;
    }
    Some(u64::from_le_bytes(frame[..8].try_into().ok()?))
}

/// The largest body that fits both the reliable-message path and one direct
/// Ethernet memory object (Ethernet header + tagged-payload header + body).
pub const MAX_RPC_PAYLOAD: usize = {
    let relmsg = crate::relmsg::MAX_MSG - 1;
    let direct = 4096 - 14 - catten_graft::wire::TAGGED_PAYLOAD_HEADER_SIZE;
    if relmsg < direct {
        relmsg
    } else {
        direct
    }
};

/// Inbound Raft RPC request decoded from a relmsg message.
pub enum InboundRpc {
    VoteRequest(catten_graft::types::VoteRequest),
    AppendEntries(catten_graft::types::AppendEntriesRequest),
    InstallSnapshot(catten_graft::types::InstallSnapshotRequest),
}

/// A tagged, encoded Raft RPC queued for a peer: (type tag, protobuf bytes).
type OutboundRpc = (u8, Vec<u8>);

struct PendingSend {
    call: u64,
    tag: u8,
}

pub struct RelmsgRaftTransport {
    relmsg_conn: spin::Mutex<u64>,
    /// Optional direct-net send path: when bound (the raft service's own
    /// EtherType), frames go straight to the NIC instead of through the
    /// reliable-message layer (which is single-consumer, owned by the dns).
    net_conn: spin::Mutex<u64>,
    src_mac: spin::Mutex<[u8; 6]>,
    ethertype: spin::Mutex<u16>,
    peer_macs: spin::Mutex<BTreeMap<String, [u8; 6]>>,
    /// Outbound RPCs queued per peer.
    outbound: spin::Mutex<BTreeMap<String, Vec<OutboundRpc>>>,
    /// Outstanding relmsg `OP_SEND` call caps per peer (0 = none).
    pending_sends: spin::Mutex<BTreeMap<String, PendingSend>>,
    acknowledged_by_tag: spin::Mutex<BTreeMap<u8, u64>>,
    acknowledged_by_peer_tag: spin::Mutex<BTreeMap<(String, u8), u64>>,
    received_responses: spin::Mutex<Vec<RpcCompletion>>,
    current_millis: spin::Mutex<u64>,
}

impl RelmsgRaftTransport {
    pub fn new(relmsg_conn: u64) -> Self {
        Self {
            relmsg_conn: spin::Mutex::new(relmsg_conn),
            net_conn: spin::Mutex::new(0),
            src_mac: spin::Mutex::new([0u8; 6]),
            ethertype: spin::Mutex::new(0),
            peer_macs: spin::Mutex::new(BTreeMap::new()),
            outbound: spin::Mutex::new(BTreeMap::new()),
            pending_sends: spin::Mutex::new(BTreeMap::new()),
            acknowledged_by_tag: spin::Mutex::new(BTreeMap::new()),
            acknowledged_by_peer_tag: spin::Mutex::new(BTreeMap::new()),
            received_responses: spin::Mutex::new(Vec::new()),
            current_millis: spin::Mutex::new(0),
        }
    }

    /// Bind (or rebind) the relmsg connection once the name service resolves
    /// it. Frames are only sent once a connection is bound.
    pub fn set_relmsg_conn(&self, conn: u64) {
        *self.relmsg_conn.lock() = conn;
    }

    /// Bind the direct-net send path: frames are then addressed to the peer
    /// MACs with `ethertype` on the NIC (used by the raft service's own
    /// EtherType; the reliable-message layer remains the default).
    pub fn set_net_send(&self, net_conn: u64, src_mac: [u8; 6], ethertype: u16) {
        *self.net_conn.lock() = net_conn;
        *self.src_mac.lock() = src_mac;
        *self.ethertype.lock() = ethertype;
    }

    pub fn net_conn(&self) -> u64 {
        *self.net_conn.lock()
    }

    pub fn add_peer(&self, peer_id: &str, mac: [u8; 6]) {
        self.peer_macs.lock().insert(peer_id.to_string(), mac);
    }

    pub fn remove_peer(&self, peer_id: &str) {
        self.peer_macs.lock().remove(peer_id);
        self.outbound.lock().remove(peer_id);
        if let Some(pending) = self.pending_sends.lock().remove(peer_id)
            && pending.call != 0
        {
            ipc_close(pending.call);
        }
    }

    pub fn has_peer(&self, peer_id: &str) -> bool {
        self.peer_macs.lock().contains_key(peer_id)
    }

    pub fn peer_count(&self) -> usize {
        self.peer_macs.lock().len()
    }

    pub fn pending_send_count(&self) -> usize {
        self.pending_sends.lock().values().filter(|send| send.call != 0).count()
    }

    pub fn outbound_count(&self) -> usize {
        self.outbound.lock().values().map(Vec::len).sum()
    }

    /// The MAC the transport currently routes `peer_id` to.
    pub fn mac_for_peer(&self, peer_id: &str) -> Option<[u8; 6]> {
        self.peer_macs.lock().get(peer_id).copied()
    }

    /// The peer id whose MAC is `mac`.
    pub fn peer_id_for_mac(&self, mac: &[u8; 6]) -> Option<String> {
        self.peer_macs.lock().iter().find_map(|(id, peer_mac)| {
            if peer_mac == mac {
                Some(id.clone())
            } else {
                None
            }
        })
    }

    /// Enqueue an arbitrary tagged message for `peer_id`, serialized with the
    /// Raft RPCs on the same per-peer outbound path. Used by the distributed
    /// name service to relay remote invocations.
    pub fn send_message(&self, peer_id: &str, tag: u8, payload: Vec<u8>) {
        self.queue_rpc(peer_id, tag, payload);
    }

    /// Encode `payload` as a tagged outbound RPC and queue it for `peer_id`.
    fn queue_rpc(&self, peer_id: &str, tag: u8, payload: Vec<u8>) {
        let mut outbound = self.outbound.lock();
        let queue = outbound.entry(peer_id.to_string()).or_default();
        // Coalesce queued AppendEntries requests. A newer unsent request
        // supersedes an older heartbeat/replication attempt. Responses are
        // deliberately not coalesced: they report the outcome of distinct
        // requests and discarding one can hide a rejection or successful
        // replication transition from the leader.
        if queue.iter().any(|queued| queued.0 == tag && queued.1 == payload) {
            return;
        }
        if tag == TAG_APPEND_REQUEST
            && queue.last().is_some_and(|(queued_tag, _)| *queued_tag == tag)
        {
            *queue.last_mut().expect("queue last") = (tag, payload);
        } else if !matches!(tag, TAG_APPEND_REQUEST | TAG_APPEND_RESPONSE) {
            // Preserve the order of application/control messages, but place
            // them ahead of queued, supersedable AppendEntries traffic. An
            // append already in flight remains non-preemptible; relmsg's
            // bounded retry lease limits that delay.
            let position = queue
                .iter()
                .position(|(queued_tag, _)| {
                    matches!(*queued_tag, TAG_APPEND_REQUEST | TAG_APPEND_RESPONSE)
                })
                .unwrap_or(queue.len());
            queue.insert(position, (tag, payload));
        } else {
            queue.push((tag, payload));
        }
        drop(outbound);
        self.drain_outbound();
    }

    /// Transmit the next queued RPC per peer that has no send in flight.
    pub fn drain_outbound(&self) {
        let mut outbound = self.outbound.lock();
        let mut pending = self.pending_sends.lock();
        for (peer_id, queue) in outbound.iter_mut() {
            if pending.get(peer_id).is_some_and(|send| send.call != 0) {
                continue;
            }
            let Some((tag, payload)) = queue.first() else {
                continue;
            };
            let (tag, payload) = (*tag, payload.clone());
            let Some(mac) = self.peer_macs.lock().get(peer_id).copied() else {
                continue;
            };
            let net_conn = *self.net_conn.lock();
            let call = if net_conn != 0 {
                send_payload_net(
                    net_conn,
                    &self.src_mac.lock(),
                    &mac,
                    *self.ethertype.lock(),
                    tag,
                    &payload,
                )
            } else {
                send_payload(*self.relmsg_conn.lock(), &mac, tag, &payload)
            };
            let Some(call) = call else {
                continue;
            };
            queue.remove(0);
            pending.insert(
                peer_id.clone(),
                PendingSend {
                    call,
                    tag,
                },
            );
        }
    }

    /// Close relmsg `OP_SEND` calls that have been acknowledged.
    pub fn reap_acks(&self) {
        let mut completed = Vec::new();
        {
            let mut pending = self.pending_sends.lock();
            for (peer_id, send) in pending.iter_mut() {
                if send.call == 0 {
                    continue;
                }
                let (status, result, _connection, _memory) = ipc_reply_poll_with_memory(send.call);
                if status != 1 {
                    ipc_close(send.call);
                    send.call = 0;
                    if status == 0 && result <= crate::relmsg::MAX_MSG as u64 {
                        let mut counts = self.acknowledged_by_tag.lock();
                        let count = counts.entry(send.tag).or_default();
                        *count = count.saturating_add(1);
                        drop(counts);
                        let mut peer_counts = self.acknowledged_by_peer_tag.lock();
                        let count = peer_counts.entry((peer_id.clone(), send.tag)).or_default();
                        *count = count.saturating_add(1);
                    }
                    completed.push(peer_id.clone());
                }
            }
        }
        let _ = completed;
    }

    /// Number of messages with `tag` acknowledged by the remote relmsg
    /// instance. This is transport delivery, not application processing.
    pub fn acknowledged_count(&self, tag: u8) -> u64 {
        self.acknowledged_by_tag.lock().get(&tag).copied().unwrap_or(0)
    }

    /// Number of messages with `tag` acknowledged by a particular remote
    /// relmsg instance. The per-peer count lets callers determine when a
    /// specific queued reply has become transport-settled.
    pub fn acknowledged_count_for(&self, peer_id: &str, tag: u8) -> u64 {
        self.acknowledged_by_peer_tag.lock().get(&(peer_id.to_string(), tag)).copied().unwrap_or(0)
    }

    /// Send a tagged RPC response to a peer MAC, routed through the per-peer
    /// outbound queue so it never races a pending request send to the same
    /// peer (relmsg allows one in-flight `OP_SEND` per peer).
    pub fn send_response(&self, peer_mac: [u8; 6], tag: u8, payload: Vec<u8>) {
        if let Some(peer_id) = self.peer_id_for_mac(&peer_mac) {
            self.queue_rpc(&peer_id, tag, payload);
        }
    }

    /// Decode an inbound relmsg message. Requests are returned for the reactor
    /// to drive `RaftNode::handle_*`; responses are buffered for
    /// `poll_completions` and `None` is returned.
    pub fn decode_inbound(&self, source_mac: &[u8; 6], frame: &[u8]) -> Option<InboundRpc> {
        let (&tag, payload) = frame.split_first()?;
        self.decode_inbound_parts(source_mac, tag, payload)
    }

    /// Decode an inbound Raft message whose transport framing has already
    /// separated the message tag from its exact, unpadded body.
    pub fn decode_inbound_parts(
        &self,
        source_mac: &[u8; 6],
        tag: u8,
        payload: &[u8],
    ) -> Option<InboundRpc> {
        match tag {
            TAG_VOTE_REQUEST => decode_vote_request(payload).ok().map(InboundRpc::VoteRequest),
            TAG_APPEND_REQUEST => {
                decode_append_request(payload).ok().map(InboundRpc::AppendEntries)
            }
            TAG_SNAPSHOT_REQUEST => {
                decode_snapshot_request(payload).ok().map(InboundRpc::InstallSnapshot)
            }
            TAG_VOTE_RESPONSE => {
                let response = decode_vote_response(payload).ok()?;
                let peer_id = self.peer_id_for_mac(source_mac)?;
                self.received_responses.lock().push(RpcCompletion::Vote {
                    peer_id,
                    response,
                });
                None
            }
            TAG_APPEND_RESPONSE => {
                let response = decode_append_response(payload).ok()?;
                let peer_id = self.peer_id_for_mac(source_mac)?;
                self.received_responses.lock().push(RpcCompletion::AppendEntries {
                    peer_id,
                    response,
                });
                None
            }
            TAG_SNAPSHOT_RESPONSE => {
                let response = decode_snapshot_response(payload).ok()?;
                let peer_id = self.peer_id_for_mac(source_mac)?;
                let sent_next_offset = response.next_offset;
                let sent_done = response.done;
                self.received_responses.lock().push(RpcCompletion::InstallSnapshot {
                    peer_id,
                    response,
                    sent_next_offset,
                    sent_done,
                });
                None
            }
            _ => None,
        }
    }

    fn encode_append(&self, rpc: &AppendEntriesRpc<'_>) -> Vec<u8> {
        let mut entries = rpc.entries.clone();
        loop {
            let request = catten_graft::types::AppendEntriesRequest {
                term: rpc.term,
                leader_id: rpc.leader_id.to_string(),
                prev_log_index: rpc.prev_log_index,
                prev_log_term: rpc.prev_log_term,
                leader_commit: rpc.leader_commit,
                entries: entries.clone(),
            };
            match encode_append_request(&request) {
                Ok(payload) if payload.len() <= MAX_RPC_PAYLOAD => return payload,
                Ok(_) => {}
                Err(_) => {}
            }
            if entries.pop().is_none() {
                // Nothing fits; the peer gets an empty (heartbeat) append.
                let request = catten_graft::types::AppendEntriesRequest {
                    term: rpc.term,
                    leader_id: rpc.leader_id.to_string(),
                    prev_log_index: rpc.prev_log_index,
                    prev_log_term: rpc.prev_log_term,
                    leader_commit: rpc.leader_commit,
                    entries: Vec::new(),
                };
                return encode_append_request(&request).unwrap_or_default();
            }
        }
    }
}

impl Default for RelmsgRaftTransport {
    fn default() -> Self {
        Self::new(0)
    }
}

impl RaftTransport for RelmsgRaftTransport {
    fn set_current_millis(&self, current_millis: u64) {
        *self.current_millis.lock() = current_millis;
    }

    fn send_vote_request(
        &self,
        peer: &Peer,
        term: u64,
        candidate_id: &str,
        last_log_index: u64,
        last_log_term: u64,
    ) {
        let request = catten_graft::types::VoteRequest {
            term,
            candidate_id: candidate_id.to_string(),
            last_log_index,
            last_log_term,
        };
        if let Ok(payload) = encode_vote_request(&request)
            && payload.len() <= MAX_RPC_PAYLOAD
        {
            self.queue_rpc(&peer.id, TAG_VOTE_REQUEST, payload);
        }
    }

    fn send_append_entries(&self, rpc: AppendEntriesRpc<'_>) {
        let payload = self.encode_append(&rpc);
        if !payload.is_empty() {
            self.queue_rpc(&rpc.peer.id, TAG_APPEND_REQUEST, payload);
        }
    }

    fn send_install_snapshot(&self, rpc: InstallSnapshotRpc<'_>) {
        let request = catten_graft::types::InstallSnapshotRequest {
            term: rpc.term,
            leader_id: rpc.leader_id.to_string(),
            last_included_index: rpc.last_included_index,
            last_included_term: rpc.last_included_term,
            offset: rpc.offset,
            data: rpc.data,
            done: rpc.done,
        };
        if let Ok(payload) = encode_snapshot_request(&request)
            && payload.len() <= MAX_RPC_PAYLOAD
        {
            self.queue_rpc(&rpc.peer.id, TAG_SNAPSHOT_REQUEST, payload);
        }
    }

    fn broadcast_heartbeat_complete(&self) {}

    fn poll_completions(&self) -> Vec<RpcCompletion> {
        core::mem::take(&mut *self.received_responses.lock())
    }
}

/// Write `tag + payload` into a memory object and submit `relmsg::OP_SEND` to
/// `mac`. Returns the pending call cap, or `None` on failure.
/// Send a raw Ethernet frame directly on the NIC: the destination MAC, the
/// transport's own source MAC, the given EtherType, then the tagged payload.
fn send_payload_net(
    net_conn: u64,
    src_mac: &[u8; 6],
    dst_mac: &[u8; 6],
    ethertype: u16,
    tag: u8,
    payload: &[u8],
) -> Option<u64> {
    let tagged_header = catten_graft::wire::build_tagged_payload_header(tag, payload.len()).ok()?;
    let len = 14 + tagged_header.len() + payload.len();
    if len > 4096 {
        return None;
    }
    let cap = memory_alloc(1);
    if cap == 0 {
        return None;
    }
    if memory_map(cap, SCRATCH, true) != 0 {
        memory_close(cap);
        return None;
    }
    unsafe {
        let base = SCRATCH as *mut u8;
        core::ptr::copy_nonoverlapping(dst_mac.as_ptr(), base, 6);
        core::ptr::copy_nonoverlapping(src_mac.as_ptr(), base.add(6), 6);
        base.add(12).copy_from(ethertype.to_be_bytes().as_ptr(), 2);
        core::ptr::copy_nonoverlapping(tagged_header.as_ptr(), base.add(14), tagged_header.len());
        core::ptr::copy_nonoverlapping(
            payload.as_ptr(),
            base.add(14 + tagged_header.len()),
            payload.len(),
        );
    }
    memory_unmap(cap);
    let call = ipc_scalar_call_move(net_conn, crate::net::OP_SEND, len as u64, cap);
    if call == 0 {
        memory_close(cap);
        return None;
    }
    Some(call)
}

fn send_payload(relmsg_conn: u64, mac: &[u8; 6], tag: u8, payload: &[u8]) -> Option<u64> {
    let len = payload.len() + 1;
    if len > crate::relmsg::MAX_MSG {
        return None;
    }
    let pages = len.div_ceil(4096);
    let cap = memory_alloc(pages);
    if cap == 0 {
        return None;
    }
    if memory_map(cap, SCRATCH, true) != 0 {
        memory_close(cap);
        return None;
    }
    unsafe {
        (SCRATCH as *mut u8).write(tag);
        core::ptr::copy_nonoverlapping(
            payload.as_ptr(),
            (SCRATCH as *mut u8).add(1),
            payload.len(),
        );
    }
    memory_unmap(cap);
    let destination = charlotte_protocol_msg::pack_address_and_len(*mac, len as u16);
    let call = ipc_scalar_call_move(relmsg_conn, crate::relmsg::OP_SEND, destination, cap);
    if call == 0 {
        memory_close(cap);
        None
    } else {
        Some(call)
    }
}
