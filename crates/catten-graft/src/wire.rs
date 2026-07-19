#[repr(C)]
#[derive(Clone, Copy)]
pub struct VoteRequestWire {
    pub term: u64,
    pub candidate_id_len: u32,
    pub candidate_id_off: u32,
    pub last_log_index: u64,
    pub last_log_term: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VoteResponseWire {
    pub term: u64,
    pub vote_granted: u8,
    pub _pad: [u8; 7],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AppendEntriesRequestWire {
    pub term: u64,
    pub leader_id_len: u32,
    pub leader_id_off: u32,
    pub prev_log_index: u64,
    pub prev_log_term: u64,
    pub leader_commit: u64,
    pub entry_count: u32,
    pub entries_data_off: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AppendEntriesResponseWire {
    pub term: u64,
    pub success: u8,
    pub _pad: [u8; 7],
    pub match_index: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InstallSnapshotRequestWire {
    pub term: u64,
    pub leader_id_len: u32,
    pub leader_id_off: u32,
    pub last_included_index: u64,
    pub last_included_term: u64,
    pub offset: u64,
    pub data_len: u32,
    pub data_off: u32,
    pub done: u8,
    pub _pad: [u8; 7],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InstallSnapshotResponseWire {
    pub term: u64,
}

pub const VOTE_REQ_TAG: u32 = 1;
pub const VOTE_RESP_TAG: u32 = 2;
pub const APPEND_REQ_TAG: u32 = 3;
pub const APPEND_RESP_TAG: u32 = 4;
pub const SNAPSHOT_REQ_TAG: u32 = 5;
pub const SNAPSHOT_RESP_TAG: u32 = 6;

pub const RAFT_RPC_MEMORY_PAGES: usize = 1;
pub const RAFT_RPC_MEMORY_SIZE: usize = 4096;

// 0x800000 is occupied by the current EL0 loader layout for this binary on
// AArch64; use a dedicated page beyond the transport scratch mapping.
pub const SCRATCH_VADDR: usize = 0x0000_0000_0082_0000;

use alloc::{
    string::String,
    vec::Vec,
};

use crate::types::{
    AppendEntriesRequest,
    AppendEntriesResponse,
    InstallSnapshotRequest,
    InstallSnapshotResponse,
    LogEntry,
    VoteRequest,
    VoteResponse,
};

const MAGIC: [u8; 4] = *b"RFT1";
const PREFIX_SIZE: usize = 8;
const KIND_VOTE_REQUEST: u8 = 1;
const KIND_VOTE_RESPONSE: u8 = 2;
const KIND_APPEND_REQUEST: u8 = 3;
const KIND_APPEND_RESPONSE: u8 = 4;
const KIND_SNAPSHOT_REQUEST: u8 = 5;
const KIND_SNAPSHOT_RESPONSE: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    InvalidHeader,
    WrongKind,
    Truncated,
    Oversized,
    InvalidUtf8,
}

fn begin(kind: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(RAFT_RPC_MEMORY_SIZE);
    out.extend_from_slice(&MAGIC);
    out.push(kind);
    out.push(1);
    out.extend_from_slice(&[0, 0]);
    out
}

fn finish(mut out: Vec<u8>) -> Result<Vec<u8>, WireError> {
    if out.len() > RAFT_RPC_MEMORY_SIZE || out.len() > u16::MAX as usize {
        return Err(WireError::Oversized);
    }
    let len = out.len() as u16;
    out[6..8].copy_from_slice(&len.to_le_bytes());
    Ok(out)
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), WireError> {
    let len = u16::try_from(bytes.len()).map_err(|_| WireError::Oversized)?;
    put_u16(out, len);
    out.extend_from_slice(bytes);
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], kind: u8) -> Result<Self, WireError> {
        if bytes.len() < PREFIX_SIZE || bytes[..4] != MAGIC || bytes[4] != kind || bytes[5] != 1 {
            return Err(WireError::InvalidHeader);
        }
        let declared = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
        if declared < PREFIX_SIZE || declared > bytes.len() || declared > RAFT_RPC_MEMORY_SIZE {
            return Err(WireError::Truncated);
        }
        Ok(Self {
            bytes: &bytes[..declared],
            offset: PREFIX_SIZE,
        })
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], WireError> {
        let end = self.offset.checked_add(len).ok_or(WireError::Oversized)?;
        let value = self.bytes.get(self.offset..end).ok_or(WireError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, WireError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, WireError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, WireError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    fn bytes(&mut self) -> Result<&'a [u8], WireError> {
        let len = self.u16()? as usize;
        self.take(len)
    }

    fn string(&mut self) -> Result<String, WireError> {
        let bytes = self.bytes()?;
        let value = core::str::from_utf8(bytes).map_err(|_| WireError::InvalidUtf8)?;
        Ok(String::from(value))
    }

    fn finish(self) -> Result<(), WireError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(WireError::InvalidHeader)
        }
    }
}

pub fn encode_vote_request(req: &VoteRequest) -> Result<Vec<u8>, WireError> {
    let mut out = begin(KIND_VOTE_REQUEST);
    put_u64(&mut out, req.term);
    put_u64(&mut out, req.last_log_index);
    put_u64(&mut out, req.last_log_term);
    put_bytes(&mut out, req.candidate_id.as_bytes())?;
    finish(out)
}

pub fn decode_vote_request(bytes: &[u8]) -> Result<VoteRequest, WireError> {
    let mut reader = Reader::new(bytes, KIND_VOTE_REQUEST)?;
    let value = VoteRequest {
        term: reader.u64()?,
        last_log_index: reader.u64()?,
        last_log_term: reader.u64()?,
        candidate_id: reader.string()?,
    };
    reader.finish()?;
    Ok(value)
}

pub fn encode_vote_response(resp: &VoteResponse) -> Result<Vec<u8>, WireError> {
    let mut out = begin(KIND_VOTE_RESPONSE);
    put_u64(&mut out, resp.term);
    out.push(u8::from(resp.vote_granted));
    finish(out)
}

pub fn decode_vote_response(bytes: &[u8]) -> Result<VoteResponse, WireError> {
    let mut reader = Reader::new(bytes, KIND_VOTE_RESPONSE)?;
    let value = VoteResponse {
        term: reader.u64()?,
        vote_granted: reader.u8()? != 0,
    };
    reader.finish()?;
    Ok(value)
}

pub fn encode_append_request(req: &AppendEntriesRequest) -> Result<Vec<u8>, WireError> {
    let mut out = begin(KIND_APPEND_REQUEST);
    put_u64(&mut out, req.term);
    put_u64(&mut out, req.prev_log_index);
    put_u64(&mut out, req.prev_log_term);
    put_u64(&mut out, req.leader_commit);
    put_bytes(&mut out, req.leader_id.as_bytes())?;
    let count = u16::try_from(req.entries.len()).map_err(|_| WireError::Oversized)?;
    put_u16(&mut out, count);
    for entry in &req.entries {
        put_u64(&mut out, entry.term);
        put_bytes(&mut out, entry.peer_id.as_bytes())?;
        put_bytes(&mut out, &entry.data)?;
    }
    finish(out)
}

pub fn decode_append_request(bytes: &[u8]) -> Result<AppendEntriesRequest, WireError> {
    let mut reader = Reader::new(bytes, KIND_APPEND_REQUEST)?;
    let term = reader.u64()?;
    let prev_log_index = reader.u64()?;
    let prev_log_term = reader.u64()?;
    let leader_commit = reader.u64()?;
    let leader_id = reader.string()?;
    let count = reader.u16()? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(LogEntry {
            term: reader.u64()?,
            peer_id: reader.string()?,
            data: reader.bytes()?.to_vec(),
        });
    }
    reader.finish()?;
    Ok(AppendEntriesRequest {
        term,
        leader_id,
        prev_log_index,
        prev_log_term,
        leader_commit,
        entries,
    })
}

pub fn encode_append_response(resp: &AppendEntriesResponse) -> Result<Vec<u8>, WireError> {
    let mut out = begin(KIND_APPEND_RESPONSE);
    put_u64(&mut out, resp.term);
    out.push(u8::from(resp.success));
    put_u64(&mut out, resp.match_index);
    finish(out)
}

pub fn decode_append_response(bytes: &[u8]) -> Result<AppendEntriesResponse, WireError> {
    let mut reader = Reader::new(bytes, KIND_APPEND_RESPONSE)?;
    let value = AppendEntriesResponse {
        term: reader.u64()?,
        success: reader.u8()? != 0,
        match_index: reader.u64()?,
    };
    reader.finish()?;
    Ok(value)
}

pub fn encode_snapshot_request(req: &InstallSnapshotRequest) -> Result<Vec<u8>, WireError> {
    let mut out = begin(KIND_SNAPSHOT_REQUEST);
    put_u64(&mut out, req.term);
    put_u64(&mut out, req.last_included_index);
    put_u64(&mut out, req.last_included_term);
    put_u64(&mut out, req.offset);
    out.push(u8::from(req.done));
    put_bytes(&mut out, req.leader_id.as_bytes())?;
    let data_len = u32::try_from(req.data.len()).map_err(|_| WireError::Oversized)?;
    put_u32(&mut out, data_len);
    out.extend_from_slice(&req.data);
    finish(out)
}

pub fn decode_snapshot_request(bytes: &[u8]) -> Result<InstallSnapshotRequest, WireError> {
    let mut reader = Reader::new(bytes, KIND_SNAPSHOT_REQUEST)?;
    let term = reader.u64()?;
    let last_included_index = reader.u64()?;
    let last_included_term = reader.u64()?;
    let offset = reader.u64()?;
    let done = reader.u8()? != 0;
    let leader_id = reader.string()?;
    let data_len = reader.u32()? as usize;
    let data = reader.take(data_len)?.to_vec();
    reader.finish()?;
    Ok(InstallSnapshotRequest {
        term,
        leader_id,
        last_included_index,
        last_included_term,
        offset,
        data,
        done,
    })
}

pub fn encode_snapshot_response(resp: &InstallSnapshotResponse) -> Result<Vec<u8>, WireError> {
    let mut out = begin(KIND_SNAPSHOT_RESPONSE);
    put_u64(&mut out, resp.term);
    out.push(u8::from(resp.success));
    put_u64(&mut out, resp.last_included_index);
    put_u64(&mut out, resp.next_offset);
    out.push(u8::from(resp.done));
    finish(out)
}

pub fn decode_snapshot_response(bytes: &[u8]) -> Result<InstallSnapshotResponse, WireError> {
    let mut reader = Reader::new(bytes, KIND_SNAPSHOT_RESPONSE)?;
    let value = InstallSnapshotResponse {
        term: reader.u64()?,
        success: reader.u8()? != 0,
        last_included_index: reader.u64()?,
        next_offset: reader.u64()?,
        done: reader.u8()? != 0,
    };
    reader.finish()?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use alloc::{
        string::String,
        vec,
    };

    use super::*;

    #[test]
    fn raft_messages_round_trip() {
        let vote = VoteRequest {
            term: 7,
            candidate_id: String::from("node-a"),
            last_log_index: 9,
            last_log_term: 6,
        };
        let decoded = decode_vote_request(&encode_vote_request(&vote).unwrap()).unwrap();
        assert_eq!(decoded.term, vote.term);
        assert_eq!(decoded.candidate_id, vote.candidate_id);
        assert_eq!(decoded.last_log_index, vote.last_log_index);

        let append = AppendEntriesRequest {
            term: 8,
            leader_id: String::from("node-b"),
            prev_log_index: 4,
            prev_log_term: 3,
            leader_commit: 4,
            entries: vec![LogEntry::new(8, String::from("node-b"), vec![1, 2, 3])],
        };
        let decoded = decode_append_request(&encode_append_request(&append).unwrap()).unwrap();
        assert_eq!(decoded.term, append.term);
        assert_eq!(decoded.entries, append.entries);

        let snapshot = InstallSnapshotRequest {
            term: 9,
            leader_id: String::from("node-c"),
            last_included_index: 20,
            last_included_term: 8,
            offset: 128,
            data: vec![4, 5, 6],
            done: true,
        };
        let decoded =
            decode_snapshot_request(&encode_snapshot_request(&snapshot).unwrap()).unwrap();
        assert_eq!(decoded.data, snapshot.data);
        assert_eq!(decoded.leader_id, snapshot.leader_id);
        assert!(decoded.done);
    }

    #[test]
    fn malformed_and_oversized_messages_are_rejected() {
        assert!(matches!(decode_vote_request(b"bad"), Err(WireError::InvalidHeader)));
        let request = InstallSnapshotRequest {
            term: 1,
            leader_id: String::from("n1"),
            last_included_index: 1,
            last_included_term: 1,
            offset: 0,
            data: vec![0; RAFT_RPC_MEMORY_SIZE],
            done: true,
        };
        assert_eq!(encode_snapshot_request(&request), Err(WireError::Oversized));
    }
}
