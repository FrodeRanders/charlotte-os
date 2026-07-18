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

pub const SCRATCH_VADDR: usize = 0x0000_0000_0080_0000;
