//! Raft peer transport over the reliable message layer (`relmsg`).
//!
//! Implements `catten_graft::RaftTransport` by carrying Vote/AppendEntries/
//! InstallSnapshot RPCs as `relmsg` messages addressed to peer node MACs,
//! routed through the frame demultiplexer to the NIC. Each message is prefixed
//! with a one-byte request/response type tag followed by the encoded protobuf.
//!
//! - Requests are handed to the owning reactor (which drives
//!   `RaftNode::handle_*`) and answered with a tagged response back to the
//!   source MAC.
//! - Responses are buffered and surfaced through `poll_completions`, exactly
//!   like `CharlotteTransport`.
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

/// relmsg caps a message payload at `relmsg::MAX_MSG`; one byte is the tag.
pub const MAX_RPC_PAYLOAD: usize = crate::relmsg::MAX_MSG - 1;

/// Inbound Raft RPC request decoded from a relmsg message.
pub enum InboundRpc {
    VoteRequest(catten_graft::types::VoteRequest),
    AppendEntries(catten_graft::types::AppendEntriesRequest),
    InstallSnapshot(catten_graft::types::InstallSnapshotRequest),
}

/// A tagged, encoded Raft RPC queued for a peer: (type tag, protobuf bytes).
type OutboundRpc = (u8, Vec<u8>);

pub struct RelmsgRaftTransport {
    relmsg_conn: u64,
    peer_macs: spin::Mutex<BTreeMap<String, [u8; 6]>>,
    /// Outbound RPCs queued per peer.
    outbound: spin::Mutex<BTreeMap<String, Vec<OutboundRpc>>>,
    /// Outstanding relmsg `OP_SEND` call caps per peer (0 = none).
    pending_sends: spin::Mutex<BTreeMap<String, u64>>,
    received_responses: spin::Mutex<Vec<RpcCompletion>>,
    current_millis: spin::Mutex<u64>,
}

impl RelmsgRaftTransport {
    pub fn new(relmsg_conn: u64) -> Self {
        Self {
            relmsg_conn,
            peer_macs: spin::Mutex::new(BTreeMap::new()),
            outbound: spin::Mutex::new(BTreeMap::new()),
            pending_sends: spin::Mutex::new(BTreeMap::new()),
            received_responses: spin::Mutex::new(Vec::new()),
            current_millis: spin::Mutex::new(0),
        }
    }

    pub fn add_peer(&self, peer_id: &str, mac: [u8; 6]) {
        self.peer_macs.lock().insert(peer_id.to_string(), mac);
    }

    pub fn remove_peer(&self, peer_id: &str) {
        self.peer_macs.lock().remove(peer_id);
        self.outbound.lock().remove(peer_id);
        if let Some(call) = self.pending_sends.lock().remove(peer_id)
            && call != 0
        {
            ipc_close(call);
        }
    }

    pub fn has_peer(&self, peer_id: &str) -> bool {
        self.peer_macs.lock().contains_key(peer_id)
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
        // Coalesce queued heartbeats: a newer AppendEntries supersedes an older
        // one already waiting, so a slow ACK cannot starve other traffic
        // (relmsg allows one in-flight send per peer).
        if tag == TAG_APPEND_REQUEST
            && queue.last().is_some_and(|(queued_tag, _)| *queued_tag == tag)
        {
            *queue.last_mut().expect("queue last") = (tag, payload);
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
            if pending.get(peer_id).is_some_and(|cap| *cap != 0) {
                continue;
            }
            let Some((tag, payload)) = queue.first() else {
                continue;
            };
            let (tag, payload) = (*tag, payload.clone());
            let Some(mac) = self.peer_macs.lock().get(peer_id).copied() else {
                continue;
            };
            let Some(call) = send_payload(self.relmsg_conn, &mac, tag, &payload) else {
                continue;
            };
            queue.remove(0);
            pending.insert(peer_id.clone(), call);
        }
    }

    /// Close relmsg `OP_SEND` calls that have been acknowledged.
    pub fn reap_acks(&self) {
        let mut completed = Vec::new();
        {
            let mut pending = self.pending_sends.lock();
            for (peer_id, call) in pending.iter_mut() {
                if *call == 0 {
                    continue;
                }
                let (status, _result, _connection, _memory) = ipc_reply_poll_with_memory(*call);
                if status != 1 {
                    ipc_close(*call);
                    *call = 0;
                    completed.push(peer_id.clone());
                }
            }
        }
        let _ = completed;
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
        match tag {
            TAG_VOTE_REQUEST => {
                decode_vote_request(payload).ok().map(InboundRpc::VoteRequest)
            }
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
                // Snapshot offset/done are tracked only for the request side;
                // this transport carries no progress state back.
                self.received_responses.lock().push(RpcCompletion::InstallSnapshot {
                    peer_id,
                    response,
                    sent_next_offset: 0,
                    sent_done: false,
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
fn send_payload(relmsg_conn: u64, mac: &[u8; 6], tag: u8, payload: &[u8]) -> Option<u64> {
    let len = payload.len() + 1;
    if len > crate::relmsg::MAX_MSG {
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
