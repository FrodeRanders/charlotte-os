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
    },
}

pub trait RaftTransport {
    fn send_vote_request(
        &self,
        peer: &Peer,
        term: u64,
        candidate_id: &str,
        last_log_index: u64,
        last_log_term: u64,
    );

    fn send_append_entries(
        &self,
        peer: &Peer,
        term: u64,
        leader_id: &str,
        prev_log_index: u64,
        prev_log_term: u64,
        leader_commit: u64,
        entries: Vec<LogEntry>,
    );

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
    );

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

    fn send_append_entries(
        &self,
        _peer: &Peer,
        _term: u64,
        _leader_id: &str,
        _prev_log_index: u64,
        _prev_log_term: u64,
        _leader_commit: u64,
        _entries: Vec<LogEntry>,
    ) {
    }

    fn send_install_snapshot(
        &self,
        _peer: &Peer,
        _term: u64,
        _leader_id: &str,
        _last_included_index: u64,
        _last_included_term: u64,
        _offset: u64,
        _data: Vec<u8>,
        _done: bool,
    ) {
    }

    fn broadcast_heartbeat_complete(&self) {}
}
