//! Reactor-owned state records.
//!
//! These are data only. Transition logic lives with the phase that owns it
//! (`local_calls`, `reactor`, or the registration/publication phase in the
//! service loop).

use alloc::{
    string::String,
    vec::Vec,
};

pub(super) struct InFlightCall {
    pub(super) call_id: u64,
    pub(super) expected_peer: String,
    pub(super) expected_generation: u64,
    pub(super) reply: u64,
    pub(super) deadline: u64,
}

pub(super) struct CompletedCall {
    pub(super) caller: Vec<u8>,
    pub(super) session: u64,
    pub(super) call_id: u64,
    pub(super) result: i64,
    pub(super) peer: String,
    pub(super) settled_after_ack: u64,
}

pub(super) enum LocalCallDestination {
    Client {
        reply: u64,
    },
    Remote {
        caller: Vec<u8>,
        session: u64,
        call_id: u64,
        target_generation: u64,
        peer: String,
        settled_after_ack: u64,
    },
}

#[derive(Clone, Copy)]
pub(super) enum LocalCallStage {
    Lookup,
    Invoke,
}

/// A node-local service invocation advanced by the DNS reactor.
///
/// Lookup and invocation are deliberately asynchronous: blocking this thread
/// on an arbitrary service would also stop Raft heartbeats and relmsg receive
/// draining, allowing one failed service to destabilize the cluster catalog.
pub(super) struct PendingLocalCall {
    pub(super) completion: u64,
    pub(super) connection: u64,
    pub(super) opcode: u32,
    pub(super) arg: i64,
    pub(super) deadline: u64,
    pub(super) stage: LocalCallStage,
    pub(super) destination: LocalCallDestination,
}

pub(super) enum PendingQueryKind {
    Lookup {
        reply: u64,
        name: Vec<u8>,
    },
    Call {
        reply: u64,
        name: Vec<u8>,
        opcode: u32,
        arg: i64,
    },
    Deploy {
        reply: u64,
    },
    Release {
        reply: u64,
    },
    Operations {
        reply: u64,
    },
}

pub(super) struct PendingQuery {
    pub(super) query_id: u64,
    pub(super) expected_leader: String,
    pub(super) deadline: u64,
    pub(super) kind: PendingQueryKind,
}

pub(super) enum PendingRegistration {
    Prepare {
        log_index: u64,
        reply: u64,
        name: Vec<u8>,
        connection: u64,
        existing_local_generation: u64,
    },
    Activate {
        log_index: u64,
        reply: u64,
        name: Vec<u8>,
        generation: u64,
        connection: u64,
        local_generation: u64,
    },
    Unregister {
        log_index: u64,
        reply: u64,
        name: Vec<u8>,
        expected_generation: u64,
        local_generation: u64,
        automatic_term: Option<u64>,
    },
    Deploy {
        log_index: u64,
        reply: u64,
    },
    /// Leader-side: a follower relayed a deployment after its local
    /// administration service verified the signed descriptor.
    RemoteDeploy {
        log_index: u64,
        peer: String,
        session: u64,
        request_id: u64,
    },
    /// Leader-side: a follower relayed an already verified signed release.
    RemoteRelease {
        log_index: u64,
        peer: String,
        session: u64,
        request_id: u64,
    },
    /// Leader-side: a follower relayed an operational admission proof. The
    /// leader reverified it before submitting the compact command.
    RemoteOperations {
        log_index: u64,
        peer: String,
        session: u64,
        request_id: u64,
    },
    /// Leader-side: the key ceremony committed the cluster public key; the
    /// reply reports the committed key generation.
    SetKey {
        log_index: u64,
        reply: u64,
    },
    /// Leader-side: a follower relayed a register for a service hosted on its
    /// own node; the leader committed the register half and will activate on
    /// commit.
    RemotePrepare {
        log_index: u64,
        name: Vec<u8>,
        owner: Vec<u8>,
    },
    /// Leader-side: the activate half of a remote register has committed; the
    /// generation reply is relayed back to the hosting node.
    RemoteActivate {
        log_index: u64,
        name: Vec<u8>,
        owner: Vec<u8>,
        generation: u64,
    },
    /// Follower-side: a register for a locally hosted service was relayed to
    /// the leader; the reply completes this entry and publishes the service.
    RemoteRegister {
        reply: u64,
        name: Vec<u8>,
        connection: u64,
        local_generation: u64,
    },
}

pub(super) struct LocalPublication {
    pub(super) name: Vec<u8>,
    pub(super) generation: u64,
    pub(super) local_generation: u64,
    pub(super) connection: u64,
    pub(super) close_watch: u64,
    pub(super) endpoint_closed: bool,
    pub(super) local_cleanup_submitted: bool,
    pub(super) next_unregister_attempt: u64,
}
