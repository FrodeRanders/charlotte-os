use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Voter,
    Learner,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Peer {
    pub id: String,
    pub service_name: u64,
    pub role: Role,
}

impl Peer {
    pub fn voter(id: String, service_name: u64) -> Self {
        Self {
            id,
            service_name,
            role: Role::Voter,
        }
    }

    pub fn learner(id: String, service_name: u64) -> Self {
        Self {
            id,
            service_name,
            role: Role::Learner,
        }
    }

    pub fn is_voter(&self) -> bool {
        matches!(self.role, Role::Voter)
    }

    pub fn is_learner(&self) -> bool {
        matches!(self.role, Role::Learner)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub term: u64,
    pub peer_id: String,
    pub data: Vec<u8>,
}

impl LogEntry {
    pub fn new(term: u64, peer_id: String, data: Vec<u8>) -> Self {
        Self {
            term,
            peer_id,
            data,
        }
    }

    pub fn noop(term: u64, peer_id: String) -> Self {
        Self {
            term,
            peer_id,
            data: Vec::new(),
        }
    }

    pub fn is_noop(&self) -> bool {
        self.data.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    Follower,
    Candidate,
    Leader,
}

#[derive(Debug, Clone)]
pub struct PersistentState {
    pub current_term: u64,
    pub voted_for: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VoteRequest {
    pub term: u64,
    pub candidate_id: String,
    pub last_log_index: u64,
    pub last_log_term: u64,
}

#[derive(Debug, Clone)]
pub struct VoteResponse {
    pub term: u64,
    pub vote_granted: bool,
}

#[derive(Debug, Clone)]
pub struct AppendEntriesRequest {
    pub term: u64,
    pub leader_id: String,
    pub prev_log_index: u64,
    pub prev_log_term: u64,
    pub leader_commit: u64,
    pub entries: Vec<LogEntry>,
}

#[derive(Debug, Clone)]
pub struct AppendEntriesResponse {
    pub term: u64,
    pub success: bool,
    pub match_index: u64,
}

#[derive(Debug, Clone)]
pub struct InstallSnapshotRequest {
    pub term: u64,
    pub leader_id: String,
    pub last_included_index: u64,
    pub last_included_term: u64,
    pub offset: u64,
    pub data: Vec<u8>,
    pub done: bool,
}

#[derive(Debug, Clone)]
pub struct InstallSnapshotResponse {
    pub term: u64,
}

#[derive(Debug, Clone)]
pub struct ClientCommandRequest {
    pub command: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ClientCommandResponse {
    pub success: bool,
    pub result: Vec<u8>,
    pub leader_id: Option<String>,
}

pub const RAFT_INTERFACE: u64 = {
    let mut packed = [0u8; 8];
    let bytes = b"RAFT\x00\x00\x00\x00";
    let mut i = 0;
    while i < 8 {
        packed[i] = bytes[i];
        i += 1;
    }
    u64::from_le_bytes(packed)
};

pub const RAFT_VERSION: u32 = 1;

pub const OP_VOTE_REQUEST: u32 = 1;
pub const OP_APPEND_ENTRIES: u32 = 2;
pub const OP_INSTALL_SNAPSHOT: u32 = 3;
pub const OP_CLIENT_COMMAND: u32 = 4;
pub const OP_CLIENT_QUERY: u32 = 5;
pub const OP_ADD_SERVER: u32 = 6;
pub const OP_REMOVE_SERVER: u32 = 7;
pub const OP_STATUS: u32 = 8;

pub const ERR_NOT_LEADER: i64 = -1;
pub const ERR_LOG_INCONSISTENCY: i64 = -2;
pub const ERR_STALE_TERM: i64 = -3;
pub const ERR_NOT_FOUND: i64 = -4;
