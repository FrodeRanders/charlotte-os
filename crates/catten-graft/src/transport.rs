use alloc::vec::Vec;

use crate::types::{
    AppendEntriesResponse,
    InstallSnapshotResponse,
    LogEntry,
    Peer,
    VoteResponse,
};

pub enum RpcCompletion {
    Vote {
        peer_id: alloc::string::String,
        response: VoteResponse,
    },
    AppendEntries {
        peer_id: alloc::string::String,
        response: AppendEntriesResponse,
    },
    InstallSnapshot {
        peer_id: alloc::string::String,
        response: InstallSnapshotResponse,
        sent_next_offset: u64,
        sent_done: bool,
    },
}

pub struct AppendEntriesRpc<'a> {
    pub peer: &'a Peer,
    pub term: u64,
    pub leader_id: &'a str,
    pub prev_log_index: u64,
    pub prev_log_term: u64,
    pub leader_commit: u64,
    pub entries: Vec<LogEntry>,
}

pub struct InstallSnapshotRpc<'a> {
    pub peer: &'a Peer,
    pub term: u64,
    pub leader_id: &'a str,
    pub last_included_index: u64,
    pub last_included_term: u64,
    pub offset: u64,
    pub data: Vec<u8>,
    pub done: bool,
}

pub trait RaftTransport {
    fn set_current_millis(&self, _current_millis: u64) {}

    fn send_vote_request(
        &self,
        peer: &Peer,
        term: u64,
        candidate_id: &str,
        last_log_index: u64,
        last_log_term: u64,
    );

    fn send_append_entries(&self, rpc: AppendEntriesRpc<'_>);

    fn send_install_snapshot(&self, rpc: InstallSnapshotRpc<'_>);

    fn broadcast_heartbeat_complete(&self);

    fn poll_completions(&self) -> Vec<RpcCompletion> {
        Vec::new()
    }
}

pub struct NoopTransport;

impl RaftTransport for NoopTransport {
    fn send_vote_request(
        &self,
        _peer: &Peer,
        _term: u64,
        _candidate_id: &str,
        _last_log_index: u64,
        _last_log_term: u64,
    ) {
    }

    fn send_append_entries(&self, _rpc: AppendEntriesRpc<'_>) {}

    fn send_install_snapshot(&self, _rpc: InstallSnapshotRpc<'_>) {}

    fn broadcast_heartbeat_complete(&self) {}
}
