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
/// Direct-Ethernet Raft payload prefix: one message tag followed by the
/// unpadded body length in network byte order.
pub const TAGGED_PAYLOAD_HEADER_SIZE: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    Invalid,
    Oversized,
}

/// Build the prefix used when a Raft message is carried directly in an
/// Ethernet payload. Ethernet may pad short frames; the explicit length lets
/// the receiver exclude those bytes before protobuf decoding.
pub fn build_tagged_payload_header(tag: u8, payload_len: usize) -> Result<[u8; 3], WireError> {
    let payload_len = u16::try_from(payload_len).map_err(|_| WireError::Oversized)?;
    let [high, low] = payload_len.to_be_bytes();
    Ok([tag, high, low])
}

/// Parse a directly-carried Raft payload and return its tag and exact body,
/// ignoring any Ethernet padding after the declared body length.
pub fn parse_tagged_payload(frame: &[u8]) -> Result<(u8, &[u8]), WireError> {
    let header = frame.get(..TAGGED_PAYLOAD_HEADER_SIZE).ok_or(WireError::Invalid)?;
    let payload_len = u16::from_be_bytes([header[1], header[2]]) as usize;
    let end = TAGGED_PAYLOAD_HEADER_SIZE.checked_add(payload_len).ok_or(WireError::Oversized)?;
    let payload = frame.get(TAGGED_PAYLOAD_HEADER_SIZE..end).ok_or(WireError::Invalid)?;
    Ok((header[0], payload))
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
        next_offset: signed(response.next_offset)?,
        done: response.done,
    })
}

pub fn decode_snapshot_response(bytes: &[u8]) -> Result<InstallSnapshotResponse, WireError> {
    let response = proto::InstallSnapshotResponse::decode(bytes).map_err(|_| WireError::Invalid)?;
    Ok(InstallSnapshotResponse {
        peer_id: response.peer_id,
        term: nonnegative(response.term)?,
        success: response.success,
        last_included_index: nonnegative(response.last_included_index)?,
        next_offset: nonnegative(response.next_offset)?,
        done: response.done,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_response_preserves_chunk_progress() {
        let response = InstallSnapshotResponse {
            peer_id: "node-b".into(),
            term: 9,
            success: true,
            last_included_index: 41,
            next_offset: 8192,
            done: true,
        };

        let encoded = encode_snapshot_response(&response).expect("encode snapshot response");
        let decoded = decode_snapshot_response(&encoded).expect("decode snapshot response");

        assert_eq!(decoded.peer_id, response.peer_id);
        assert_eq!(decoded.term, response.term);
        assert_eq!(decoded.success, response.success);
        assert_eq!(decoded.last_included_index, response.last_included_index);
        assert_eq!(decoded.next_offset, response.next_offset);
        assert_eq!(decoded.done, response.done);
    }

    #[test]
    fn tagged_payload_excludes_ethernet_padding() {
        let payload = b"short response";
        let mut frame =
            build_tagged_payload_header(5, payload.len()).expect("tagged header").to_vec();
        frame.extend_from_slice(payload);
        frame.resize(46, 0);

        let (tag, decoded) = parse_tagged_payload(&frame).expect("tagged payload");
        assert_eq!(tag, 5);
        assert_eq!(decoded, payload);
    }

    #[test]
    fn tagged_payload_rejects_truncation() {
        assert_eq!(parse_tagged_payload(&[5, 0]), Err(WireError::Invalid));
        assert_eq!(parse_tagged_payload(&[5, 0, 2, 1]), Err(WireError::Invalid));
    }
}
