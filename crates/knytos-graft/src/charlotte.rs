use alloc::{
    collections::BTreeMap,
    string::{
        String,
        ToString,
    },
    vec::Vec,
};

use crate::{
    transport::{
        RaftTransport,
        RpcCompletion,
    },
    types::{
        AppendEntriesRequest,
        InstallSnapshotRequest,
        Peer,
        VoteRequest,
    },
    wire::{
        RAFT_RPC_MEMORY_SIZE,
        decode_append_response,
        decode_snapshot_response,
        decode_vote_response,
        encode_append_request,
        encode_snapshot_request,
        encode_vote_request,
    },
};

const TRANSPORT_SCRATCH_VADDR: usize = 0x0000_0000_0081_0000;
const RPC_TIMEOUT_MILLIS: u64 = 1_000;

pub struct CharlotteTransport {
    peer_connections: spin::Mutex<BTreeMap<String, u64>>,
    pending_calls: spin::Mutex<Vec<PendingRpc>>,
    current_millis: spin::Mutex<u64>,
}

pub struct PendingRpc {
    call_cap: u64,
    peer_id: String,
    rpc_type: RpcType,
    term: u64,
    deadline_millis: u64,
    snapshot_next_offset: u64,
    snapshot_done: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RpcType {
    Vote,
    AppendEntries,
    InstallSnapshot,
}

impl CharlotteTransport {
    pub fn new() -> Self {
        Self {
            peer_connections: spin::Mutex::new(BTreeMap::new()),
            pending_calls: spin::Mutex::new(Vec::new()),
            current_millis: spin::Mutex::new(0),
        }
    }

    pub fn add_peer(&self, peer_id: &str, connection_cap: u64) {
        if let Some(previous) =
            self.peer_connections.lock().insert(peer_id.to_string(), connection_cap)
            && previous != connection_cap
        {
            catten_syscall::ipc_close(previous);
        }
    }

    pub fn remove_peer(&self, peer_id: &str) {
        if let Some(connection) = self.peer_connections.lock().remove(peer_id) {
            catten_syscall::ipc_close(connection);
        }
        let mut pending = self.pending_calls.lock();
        let mut cancelled = Vec::new();
        pending.retain(|call| {
            if call.peer_id == peer_id {
                cancelled.push(call.call_cap);
                false
            } else {
                true
            }
        });
        drop(pending);
        for call in cancelled {
            catten_syscall::ipc_close(call);
        }
    }

    pub fn has_peer(&self, peer_id: &str) -> bool {
        self.peer_connections.lock().contains_key(peer_id)
    }

    fn connection(&self, peer_id: &str) -> Option<u64> {
        self.peer_connections.lock().get(peer_id).copied()
    }

    fn reserve_slot(&self, peer_id: &str, rpc_type: RpcType, term: u64) -> bool {
        let mut stale_caps = Vec::new();
        let mut pending = self.pending_calls.lock();
        pending.retain(|call| {
            let stale = call.peer_id == peer_id && call.rpc_type == rpc_type && call.term != term;
            if stale {
                stale_caps.push(call.call_cap);
            }
            !stale
        });
        let occupied =
            pending.iter().any(|call| call.peer_id == peer_id && call.rpc_type == rpc_type);
        drop(pending);
        for cap in stale_caps {
            catten_syscall::ipc_close(cap);
        }
        !occupied
    }

    fn submit(
        &self,
        peer: &Peer,
        rpc_type: RpcType,
        opcode: u32,
        term: u64,
        payload: &[u8],
        snapshot_progress: Option<(u64, bool)>,
    ) {
        if !self.reserve_slot(&peer.id, rpc_type, term) {
            return;
        }
        let Some(connection) = self.connection(&peer.id) else {
            return;
        };
        let Some(memory) = write_payload(payload) else {
            return;
        };
        let call_cap =
            catten_syscall::ipc_scalar_call_move(connection, opcode, payload.len() as u64, memory);
        if call_cap == 0 {
            catten_syscall::memory_close(memory);
            return;
        }
        self.pending_calls.lock().push(PendingRpc {
            call_cap,
            peer_id: peer.id.clone(),
            rpc_type,
            term,
            deadline_millis: self.current_millis.lock().saturating_add(RPC_TIMEOUT_MILLIS),
            snapshot_next_offset: snapshot_progress.map(|progress| progress.0).unwrap_or(0),
            snapshot_done: snapshot_progress.map(|progress| progress.1).unwrap_or(false),
        });
    }
}

impl Default for CharlotteTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl RaftTransport for CharlotteTransport {
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
        let request = VoteRequest {
            term,
            candidate_id: candidate_id.to_string(),
            last_log_index,
            last_log_term,
        };
        if let Ok(payload) = encode_vote_request(&request) {
            self.submit(peer, RpcType::Vote, crate::types::OP_VOTE_REQUEST, term, &payload, None);
        }
    }

    fn send_append_entries(&self, rpc: crate::transport::AppendEntriesRpc<'_>) {
        let crate::transport::AppendEntriesRpc {
            peer,
            term,
            leader_id,
            prev_log_index,
            prev_log_term,
            leader_commit,
            mut entries,
        } = rpc;
        loop {
            let request = AppendEntriesRequest {
                term,
                leader_id: leader_id.to_string(),
                prev_log_index,
                prev_log_term,
                leader_commit,
                entries: entries.clone(),
            };
            if let Ok(payload) = encode_append_request(&request) {
                self.submit(
                    peer,
                    RpcType::AppendEntries,
                    crate::types::OP_APPEND_ENTRIES,
                    term,
                    &payload,
                    None,
                );
                break;
            }
            // Retry with the largest prefix that fits. The follower response
            // advances match_index only through the transmitted prefix.
            if entries.pop().is_none() {
                break;
            }
        }
    }

    fn send_install_snapshot(&self, rpc: crate::transport::InstallSnapshotRpc<'_>) {
        let crate::transport::InstallSnapshotRpc {
            peer,
            term,
            leader_id,
            last_included_index,
            last_included_term,
            offset,
            data,
            done,
        } = rpc;
        let next_offset = offset.saturating_add(data.len() as u64);
        let request = InstallSnapshotRequest {
            term,
            leader_id: leader_id.to_string(),
            last_included_index,
            last_included_term,
            offset,
            data,
            done,
        };
        if let Ok(payload) = encode_snapshot_request(&request) {
            self.submit(
                peer,
                RpcType::InstallSnapshot,
                crate::types::OP_INSTALL_SNAPSHOT,
                term,
                &payload,
                Some((next_offset, done)),
            );
        }
    }

    fn broadcast_heartbeat_complete(&self) {}

    fn poll_completions(&self) -> Vec<RpcCompletion> {
        let pending = core::mem::take(&mut *self.pending_calls.lock());
        let current_millis = *self.current_millis.lock();
        let mut still_pending = Vec::new();
        let mut completed = Vec::new();

        for call in pending {
            if current_millis >= call.deadline_millis {
                catten_syscall::ipc_close(call.call_cap);
                continue;
            }
            let saved_call_cap = call.call_cap;
            let (status, result, _connection, memory) =
                catten_syscall::ipc_reply_poll_with_memory(unsafe {
                    core::ptr::read_volatile(&saved_call_cap)
                });
            // IPC_REPLY_POLL returns 1 while pending; the receive-status
            // namespace uses a different numeric value for PENDING.
            if status == 1 {
                still_pending.push(call);
                continue;
            }

            catten_syscall::ipc_close(unsafe { core::ptr::read_volatile(&saved_call_cap) });
            if status != 0 || memory == 0 {
                if memory != 0 {
                    catten_syscall::memory_close(memory);
                }
                continue;
            }
            let Ok(length) = usize::try_from(result) else {
                catten_syscall::memory_close(memory);
                continue;
            };
            let Some(payload) = read_payload(memory, length) else {
                continue;
            };
            let completion = match call.rpc_type {
                RpcType::Vote => {
                    decode_vote_response(&payload).ok().map(|response| RpcCompletion::Vote {
                        peer_id: call.peer_id,
                        response,
                    })
                }
                RpcType::AppendEntries => decode_append_response(&payload).ok().map(|response| {
                    RpcCompletion::AppendEntries {
                        peer_id: call.peer_id,
                        response,
                    }
                }),
                RpcType::InstallSnapshot => {
                    decode_snapshot_response(&payload).ok().map(|response| {
                        RpcCompletion::InstallSnapshot {
                            peer_id: call.peer_id,
                            response,
                            sent_next_offset: call.snapshot_next_offset,
                            sent_done: call.snapshot_done,
                        }
                    })
                }
            };
            if let Some(completion) = completion {
                completed.push(completion);
            }
        }

        self.pending_calls.lock().extend(still_pending);
        completed
    }
}

fn write_payload(payload: &[u8]) -> Option<u64> {
    if payload.len() > RAFT_RPC_MEMORY_SIZE {
        return None;
    }
    let cap = catten_syscall::memory_alloc(1);
    if cap == 0 {
        return None;
    }
    if catten_syscall::memory_map(cap, TRANSPORT_SCRATCH_VADDR, true) != 0 {
        catten_syscall::memory_close(cap);
        return None;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            payload.as_ptr(),
            TRANSPORT_SCRATCH_VADDR as *mut u8,
            payload.len(),
        );
    }
    catten_syscall::memory_unmap(cap);
    Some(cap)
}

fn read_payload(cap: u64, length: usize) -> Option<Vec<u8>> {
    if length > RAFT_RPC_MEMORY_SIZE {
        catten_syscall::memory_close(cap);
        return None;
    }
    if catten_syscall::memory_map(cap, TRANSPORT_SCRATCH_VADDR, false) != 0 {
        catten_syscall::memory_close(cap);
        return None;
    }
    let payload = unsafe {
        core::slice::from_raw_parts(TRANSPORT_SCRATCH_VADDR as *const u8, length).to_vec()
    };
    catten_syscall::memory_unmap(cap);
    catten_syscall::memory_close(cap);
    Some(payload)
}
