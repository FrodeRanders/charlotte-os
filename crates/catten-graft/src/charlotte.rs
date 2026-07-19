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
        LogEntry,
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

pub struct CharlotteTransport {
    peer_connections: spin::Mutex<BTreeMap<String, u64>>,
    pending_calls: spin::Mutex<Vec<PendingRpc>>,
}

pub struct PendingRpc {
    call_cap: u64,
    peer_id: String,
    rpc_type: RpcType,
    term: u64,
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
        }
    }

    pub fn add_peer(&self, peer_id: &str, connection_cap: u64) {
        self.peer_connections.lock().insert(peer_id.to_string(), connection_cap);
    }

    pub fn remove_peer(&self, peer_id: &str) {
        self.peer_connections.lock().remove(peer_id);
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

    fn submit(&self, peer: &Peer, rpc_type: RpcType, opcode: u32, term: u64, payload: &[u8]) {
        if !self.reserve_slot(&peer.id, rpc_type, term) {
            return;
        }
        let Some(connection) = self.connection(&peer.id) else {
            return;
        };
        let Some(memory) = write_payload(payload) else {
            return;
        };
        let call_cap = catten_syscall::ipc_scalar_call_move(connection, opcode, term, memory);
        if call_cap == 0 {
            catten_syscall::memory_close(memory);
            return;
        }
        self.pending_calls.lock().push(PendingRpc {
            call_cap,
            peer_id: peer.id.clone(),
            rpc_type,
            term,
        });
    }
}

impl Default for CharlotteTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl RaftTransport for CharlotteTransport {
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
            self.submit(peer, RpcType::Vote, crate::types::OP_VOTE_REQUEST, term, &payload);
        }
    }

    fn send_append_entries(
        &self,
        peer: &Peer,
        term: u64,
        leader_id: &str,
        prev_log_index: u64,
        prev_log_term: u64,
        leader_commit: u64,
        entries: Vec<LogEntry>,
    ) {
        let request = AppendEntriesRequest {
            term,
            leader_id: leader_id.to_string(),
            prev_log_index,
            prev_log_term,
            leader_commit,
            entries,
        };
        if let Ok(payload) = encode_append_request(&request) {
            self.submit(
                peer,
                RpcType::AppendEntries,
                crate::types::OP_APPEND_ENTRIES,
                term,
                &payload,
            );
        }
    }

    fn send_install_snapshot(
        &self,
        peer: &Peer,
        term: u64,
        leader_id: &str,
        last_included_index: u64,
        last_included_term: u64,
        offset: u64,
        data: Vec<u8>,
        done: bool,
    ) {
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
            );
        }
    }

    fn broadcast_heartbeat_complete(&self) {}

    fn poll_completions(&self) -> Vec<RpcCompletion> {
        let pending = core::mem::take(&mut *self.pending_calls.lock());
        let mut still_pending = Vec::new();
        let mut completed = Vec::new();

        for call in pending {
            let saved_call_cap = call.call_cap;
            let (status, _result, _connection, memory) =
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
            let Some(payload) = read_payload(memory) else {
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

fn read_payload(cap: u64) -> Option<Vec<u8>> {
    if catten_syscall::memory_map(cap, TRANSPORT_SCRATCH_VADDR, false) != 0 {
        catten_syscall::memory_close(cap);
        return None;
    }
    let payload = unsafe {
        core::slice::from_raw_parts(TRANSPORT_SCRATCH_VADDR as *const u8, RAFT_RPC_MEMORY_SIZE)
            .to_vec()
    };
    catten_syscall::memory_unmap(cap);
    catten_syscall::memory_close(cap);
    Some(payload)
}
