//! `no_std` subset of the shared Graft `raft.proto` contract.
//!
//! Field numbers and message shapes are identical to the Java, C++, and
//! general Rust implementations.

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct Envelope {
    #[prost(string, tag = "1")]
    pub correlation_id: alloc::string::String,
    #[prost(string, tag = "2")]
    pub r#type: alloc::string::String,
    #[prost(bytes = "vec", tag = "3")]
    pub payload: alloc::vec::Vec<u8>,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct PeerSpec {
    #[prost(string, tag = "1")]
    pub id: alloc::string::String,
    #[prost(string, tag = "2")]
    pub host: alloc::string::String,
    #[prost(int32, tag = "3")]
    pub port: i32,
    #[prost(string, tag = "4")]
    pub role: alloc::string::String,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct JointConfigurationCommand {
    #[prost(message, repeated, tag = "1")]
    pub members: alloc::vec::Vec<PeerSpec>,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct JoinPeerCommand {
    #[prost(message, optional, tag = "1")]
    pub member: Option<PeerSpec>,
}

#[derive(Clone, Copy, PartialEq, Eq, prost::Message)]
pub struct FinalizeConfigurationCommand {}

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct InternalRaftCommand {
    #[prost(oneof = "internal_raft_command::Command", tags = "1, 2, 3")]
    pub command: Option<internal_raft_command::Command>,
}

pub mod internal_raft_command {
    #[derive(Clone, PartialEq, Eq, prost::Oneof)]
    pub enum Command {
        #[prost(message, tag = "1")]
        Joint(super::JointConfigurationCommand),
        #[prost(message, tag = "2")]
        Finalize(super::FinalizeConfigurationCommand),
        #[prost(message, tag = "3")]
        Join(super::JoinPeerCommand),
    }
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct VoteRequest {
    #[prost(int64, tag = "1")]
    pub term: i64,
    #[prost(string, tag = "2")]
    pub candidate_id: alloc::string::String,
    #[prost(int64, tag = "3")]
    pub last_log_index: i64,
    #[prost(int64, tag = "4")]
    pub last_log_term: i64,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct VoteResponse {
    #[prost(string, tag = "1")]
    pub peer_id: alloc::string::String,
    #[prost(int64, tag = "2")]
    pub term: i64,
    #[prost(bool, tag = "3")]
    pub vote_granted: bool,
    #[prost(int64, tag = "4")]
    pub current_term: i64,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct LogEntry {
    #[prost(int64, tag = "1")]
    pub term: i64,
    #[prost(string, tag = "2")]
    pub peer_id: alloc::string::String,
    #[prost(bytes = "vec", tag = "3")]
    pub data: alloc::vec::Vec<u8>,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct AppendEntriesRequest {
    #[prost(int64, tag = "1")]
    pub term: i64,
    #[prost(string, tag = "2")]
    pub leader_id: alloc::string::String,
    #[prost(int64, tag = "3")]
    pub prev_log_index: i64,
    #[prost(int64, tag = "4")]
    pub prev_log_term: i64,
    #[prost(int64, tag = "5")]
    pub leader_commit: i64,
    #[prost(message, repeated, tag = "6")]
    pub entries: alloc::vec::Vec<LogEntry>,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct AppendEntriesResponse {
    #[prost(int64, tag = "1")]
    pub term: i64,
    #[prost(string, tag = "2")]
    pub peer_id: alloc::string::String,
    #[prost(bool, tag = "3")]
    pub success: bool,
    #[prost(int64, tag = "4")]
    pub match_index: i64,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct InstallSnapshotRequest {
    #[prost(int64, tag = "1")]
    pub term: i64,
    #[prost(string, tag = "2")]
    pub leader_id: alloc::string::String,
    #[prost(int64, tag = "3")]
    pub last_included_index: i64,
    #[prost(int64, tag = "4")]
    pub last_included_term: i64,
    #[prost(int64, tag = "5")]
    pub offset: i64,
    #[prost(bytes = "vec", tag = "6")]
    pub snapshot_data: alloc::vec::Vec<u8>,
    #[prost(bool, tag = "7")]
    pub done: bool,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct InstallSnapshotResponse {
    #[prost(int64, tag = "1")]
    pub term: i64,
    #[prost(string, tag = "2")]
    pub peer_id: alloc::string::String,
    #[prost(bool, tag = "3")]
    pub success: bool,
    #[prost(int64, tag = "4")]
    pub last_included_index: i64,
    /// Receiver's next expected byte offset. Tags 5 and 6 are a
    /// backwards-compatible Charlotte extension: older peers decode their
    /// protobuf defaults and therefore safely retry from offset zero.
    #[prost(int64, tag = "5")]
    pub next_offset: i64,
    #[prost(bool, tag = "6")]
    pub done: bool,
}
