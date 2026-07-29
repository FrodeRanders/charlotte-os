use alloc::vec::Vec;

use prost::Message;

use crate::{
    proto,
    types::{
        AppendEntriesRequest,
        AppendEntriesResponse,
        InstallSnapshotRequest,
        InstallSnapshotResponse,
        LogEntry,
        VoteRequest,
        VoteResponse,
    },
};

pub const RAFT_RPC_MEMORY_PAGES: usize = 1;
pub const RAFT_RPC_MEMORY_SIZE: usize = 4096;
pub const SCRATCH_VADDR: usize = 0x0000_0000_0082_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    Invalid,
    Oversized,
}

fn encode<M: Message>(message: &M) -> Result<Vec<u8>, WireError> {
    if message.encoded_len() > RAFT_RPC_MEMORY_SIZE {
        return Err(WireError::Oversized);
    }
    let mut bytes = Vec::with_capacity(message.encoded_len());
    message.encode(&mut bytes).map_err(|_| WireError::Invalid)?;
    Ok(bytes)
}

fn nonnegative(value: i64) -> Result<u64, WireError> {
    u64::try_from(value).map_err(|_| WireError::Invalid)
}

fn signed(value: u64) -> Result<i64, WireError> {
    i64::try_from(value).map_err(|_| WireError::Invalid)
}

pub fn encode_vote_request(request: &VoteRequest) -> Result<Vec<u8>, WireError> {
    encode(&proto::VoteRequest {
        term: signed(request.term)?,
        candidate_id: request.candidate_id.clone(),
        last_log_index: signed(request.last_log_index)?,
        last_log_term: signed(request.last_log_term)?,
    })
}

pub fn decode_vote_request(bytes: &[u8]) -> Result<VoteRequest, WireError> {
    let request = proto::VoteRequest::decode(bytes).map_err(|_| WireError::Invalid)?;
    Ok(VoteRequest {
        term: nonnegative(request.term)?,
        candidate_id: request.candidate_id,
        last_log_index: nonnegative(request.last_log_index)?,
        last_log_term: nonnegative(request.last_log_term)?,
    })
}

pub fn encode_vote_response(response: &VoteResponse) -> Result<Vec<u8>, WireError> {
    encode(&proto::VoteResponse {
        peer_id: response.peer_id.clone(),
        term: signed(response.term)?,
        vote_granted: response.vote_granted,
        current_term: signed(response.term)?,
    })
}

pub fn decode_vote_response(bytes: &[u8]) -> Result<VoteResponse, WireError> {
    let response = proto::VoteResponse::decode(bytes).map_err(|_| WireError::Invalid)?;
    Ok(VoteResponse {
        peer_id: response.peer_id,
        term: nonnegative(response.term.max(response.current_term))?,
        vote_granted: response.vote_granted,
    })
}

pub fn encode_append_request(request: &AppendEntriesRequest) -> Result<Vec<u8>, WireError> {
    encode(&proto::AppendEntriesRequest {
        term: signed(request.term)?,
        leader_id: request.leader_id.clone(),
        prev_log_index: signed(request.prev_log_index)?,
        prev_log_term: signed(request.prev_log_term)?,
        leader_commit: signed(request.leader_commit)?,
        entries: request
            .entries
            .iter()
            .map(|entry| {
                Ok(proto::LogEntry {
                    term: signed(entry.term)?,
                    peer_id: entry.peer_id.clone(),
                    data: entry.data.clone(),
                })
            })
            .collect::<Result<Vec<_>, WireError>>()?,
    })
}

pub fn decode_append_request(bytes: &[u8]) -> Result<AppendEntriesRequest, WireError> {
    let request = proto::AppendEntriesRequest::decode(bytes).map_err(|_| WireError::Invalid)?;
    Ok(AppendEntriesRequest {
        term: nonnegative(request.term)?,
        leader_id: request.leader_id,
        prev_log_index: nonnegative(request.prev_log_index)?,
        prev_log_term: nonnegative(request.prev_log_term)?,
        leader_commit: nonnegative(request.leader_commit)?,
        entries: request
            .entries
            .into_iter()
            .map(|entry| {
                Ok(LogEntry {
                    term: nonnegative(entry.term)?,
                    peer_id: entry.peer_id,
                    data: entry.data,
                })
            })
            .collect::<Result<Vec<_>, WireError>>()?,
    })
}

pub fn encode_append_response(response: &AppendEntriesResponse) -> Result<Vec<u8>, WireError> {
    encode(&proto::AppendEntriesResponse {
        term: signed(response.term)?,
        peer_id: response.peer_id.clone(),
        success: response.success,
        match_index: signed(response.match_index)?,
    })
}

pub fn decode_append_response(bytes: &[u8]) -> Result<AppendEntriesResponse, WireError> {
    let response = proto::AppendEntriesResponse::decode(bytes).map_err(|_| WireError::Invalid)?;
    Ok(AppendEntriesResponse {
        peer_id: response.peer_id,
        term: nonnegative(response.term)?,
        success: response.success,
        match_index: nonnegative(response.match_index)?,
    })
}

pub fn encode_snapshot_request(request: &InstallSnapshotRequest) -> Result<Vec<u8>, WireError> {
    encode(&proto::InstallSnapshotRequest {
        term: signed(request.term)?,
        leader_id: request.leader_id.clone(),
        last_included_index: signed(request.last_included_index)?,
        last_included_term: signed(request.last_included_term)?,
        offset: signed(request.offset)?,
        snapshot_data: request.data.clone(),
        done: request.done,
    })
}

pub fn decode_snapshot_request(bytes: &[u8]) -> Result<InstallSnapshotRequest, WireError> {
    let request = proto::InstallSnapshotRequest::decode(bytes).map_err(|_| WireError::Invalid)?;
    Ok(InstallSnapshotRequest {
        term: nonnegative(request.term)?,
        leader_id: request.leader_id,
        last_included_index: nonnegative(request.last_included_index)?,
        last_included_term: nonnegative(request.last_included_term)?,
        offset: nonnegative(request.offset)?,
        data: request.snapshot_data,
        done: request.done,
    })
}

pub fn encode_snapshot_response(response: &InstallSnapshotResponse) -> Result<Vec<u8>, WireError> {
    encode(&proto::InstallSnapshotResponse {
        term: signed(response.term)?,
        peer_id: response.peer_id.clone(),
        success: response.success,
        last_included_index: signed(response.last_included_index)?,
    })
}

pub fn decode_snapshot_response(bytes: &[u8]) -> Result<InstallSnapshotResponse, WireError> {
    let response = proto::InstallSnapshotResponse::decode(bytes).map_err(|_| WireError::Invalid)?;
    Ok(InstallSnapshotResponse {
        peer_id: response.peer_id,
        term: nonnegative(response.term)?,
        success: response.success,
        last_included_index: nonnegative(response.last_included_index)?,
        next_offset: 0,
        done: false,
    })
}
