use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use crate::transport::RaftTransport;
use crate::types::{Peer, LogEntry};

pub struct CharlotteTransport {
    peer_connections: spin::Mutex<BTreeMap<String, u64>>,
    pending_calls: spin::Mutex<Vec<PendingRpc>>,
}

pub struct PendingRpc {
    pub call_cap: u64,
    pub peer_id: String,
    pub rpc_type: RpcType,
    pub term: u64,
}

pub enum RpcType {
    VoteRequest,
    AppendEntriesResponse,
    InstallSnapshotResponse,
    HeartbeatAck,
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

    pub fn drain_pending(&self) -> Vec<PendingRpc> {
        core::mem::take(&mut *self.pending_calls.lock())
    }
}

impl RaftTransport for CharlotteTransport {
    fn send_vote_request(
        &self,
        peer: &Peer,
        term: u64,
        _candidate_id: &str,
        _last_log_index: u64,
        _last_log_term: u64,
    ) {
        let conn = {
            let peers = self.peer_connections.lock();
            peers.get(&peer.id).copied()
        };

        let Some(conn) = conn else { return };

        let cap = unsafe {
            catten_syscall::ipc_scalar_call(
                conn,
                crate::types::OP_VOTE_REQUEST,
                term,
            )
        };

        if cap != 0 {
            self.pending_calls.lock().push(PendingRpc {
                call_cap: cap,
                peer_id: peer.id.clone(),
                rpc_type: RpcType::VoteRequest,
                term,
            });
        }
    }

    fn send_append_entries(
        &self,
        peer: &Peer,
        term: u64,
        _leader_id: &str,
        _prev_log_index: u64,
        _prev_log_term: u64,
        _leader_commit: u64,
        _entries: Vec<LogEntry>,
    ) {
        let conn = {
            let peers = self.peer_connections.lock();
            peers.get(&peer.id).copied()
        };

        let Some(conn) = conn else { return };

        let cap = unsafe {
            catten_syscall::ipc_scalar_call(
                conn,
                crate::types::OP_APPEND_ENTRIES,
                term,
            )
        };

        if cap != 0 {
            self.pending_calls.lock().push(PendingRpc {
                call_cap: cap,
                peer_id: peer.id.clone(),
                rpc_type: RpcType::AppendEntriesResponse,
                term,
            });
        }
    }

    fn send_install_snapshot(
        &self,
        peer: &Peer,
        term: u64,
        _leader_id: &str,
        _last_included_index: u64,
        _last_included_term: u64,
        _offset: u64,
        _data: Vec<u8>,
        _done: bool,
    ) {
        let conn = {
            let peers = self.peer_connections.lock();
            peers.get(&peer.id).copied()
        };

        let Some(conn) = conn else { return };

        let cap = unsafe {
            catten_syscall::ipc_scalar_call(
                conn,
                crate::types::OP_INSTALL_SNAPSHOT,
                term,
            )
        };

        if cap != 0 {
            self.pending_calls.lock().push(PendingRpc {
                call_cap: cap,
                peer_id: peer.id.clone(),
                rpc_type: RpcType::InstallSnapshotResponse,
                term,
            });
        }
    }

    fn broadcast_heartbeat_complete(&self) {}
}

pub fn poll_pending_rpc(call_cap: u64) -> Option<(i64, u64)> {
    let (status, result, cap) = unsafe {
        catten_syscall::ipc_reply_poll(call_cap)
    };

    if status == 0 {
        unsafe {
            catten_syscall::ipc_close(call_cap);
        }
        Some((result as i64, cap))
    } else {
        None
    }
}
