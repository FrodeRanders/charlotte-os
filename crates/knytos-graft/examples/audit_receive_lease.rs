//! Regression for the former receive-time lease stale read.
//!
//! A fresh per-read log fence must reject the delayed old response.
//! Run on the host, outside the kernel's Cargo build-std configuration.
use std::sync::{
    Arc,
    Mutex,
};

use catten_graft::{
    log_store::{
        InMemoryLogStore,
        InMemoryPersistentStateStore,
    },
    membership::ClusterConfiguration,
    node::{
        RaftNode,
        RaftNodeConfig,
    },
    state_machine::{
        QueryableStateMachine,
        StateMachine,
    },
    transport::NoopTransport,
    types::{
        AppendEntriesRequest,
        AppendEntriesResponse,
        Peer,
        VoteRequest,
    },
};

struct Value(Mutex<Vec<u8>>);

impl StateMachine for Value {
    fn apply(&self, _term: u64, command: &[u8]) {
        *self.0.lock().unwrap() = command.to_vec();
    }

    fn reset(&self) {
        self.0.lock().unwrap().clear();
    }

    fn as_queryable(&self) -> Option<&dyn QueryableStateMachine> {
        Some(self)
    }
}

impl QueryableStateMachine for Value {
    fn query(&self, _query: &[u8]) -> Vec<u8> {
        self.0.lock().unwrap().clone()
    }
}

fn node(id: &str) -> RaftNode {
    RaftNode::new(RaftNodeConfig {
        me: Peer::voter(id.to_string(), 0),
        timeout_millis: 150,
        log_store: Box::new(InMemoryLogStore::new()),
        persistent_state: Box::new(InMemoryPersistentStateStore::new()),
        state_machine: Some(Box::new(Value(Mutex::new(Vec::new())))),
        cluster_configuration: ClusterConfiguration::stable(
            ["n1", "n2", "n3"].map(|id| Peer::voter(id.to_string(), 0)).to_vec(),
        ),
        transport: Arc::new(NoopTransport),
        current_millis: 0,
        snapshot_min_entries: 64,
        snapshot_chunk_bytes: 3000,
    })
    .unwrap()
}

fn elect(candidate: &mut RaftNode, voter: &mut RaftNode, now: u64) {
    candidate.start_election(now);
    let vote = voter.handle_vote_request(
        VoteRequest {
            term: candidate.current_term,
            candidate_id: candidate.me.id.clone(),
            last_log_index: candidate.log_store.last_index(),
            last_log_term: candidate.log_store.last_term(),
        },
        now,
    );
    assert!(vote.vote_granted);
    candidate.handle_vote_response(&voter.me.id, vote, now);
}

fn replicate(leader: &RaftNode, follower: &mut RaftNode, now: u64) -> AppendEntriesResponse {
    follower.handle_append_entries(
        AppendEntriesRequest {
            term: leader.current_term,
            leader_id: leader.me.id.clone(),
            prev_log_index: 0,
            prev_log_term: 0,
            leader_commit: leader.commit_index,
            entries: leader.log_store.entries_from(1),
        },
        now,
    )
}

fn main() {
    let (mut old, mut next, mut third) = (node("n1"), node("n2"), node("n3"));
    elect(&mut old, &mut next, 200);
    let first = old.submit_command(vec![1], 201).unwrap();
    let reply = replicate(&old, &mut next, 202);
    assert!(reply.success);
    old.handle_append_entries_response("n2", reply, 202);
    assert!(old.is_committed(first));

    // This honest term-one success will be delivered after n2 becomes leader.
    let delayed = replicate(&old, &mut next, 203);
    assert!(delayed.success);
    assert_eq!(delayed.term, 1);
    assert_eq!(delayed.match_index, first);

    // n1 is isolated. n2 and n3 elect a leader and commit a newer value.
    elect(&mut next, &mut third, 1_000);
    let second = next.submit_command(vec![2], 1_001).unwrap();
    let reply = replicate(&next, &mut third, 1_002);
    assert!(reply.success);
    next.handle_append_entries_response("n3", reply, 1_002);
    assert!(next.is_committed(second));
    let read = next.begin_read(Vec::new(), 1_003).unwrap();
    let reply = replicate(&next, &mut third, 1_004);
    next.handle_append_entries_response("n3", reply, 1_004);
    assert_eq!(next.handle_client_query(read), Ok(vec![2]));
    println!("new leader n2: term={} committed={} value=2", next.current_term, second);

    old.set_millis(5_000);
    let read = old.begin_read(Vec::new(), 5_000).unwrap();
    assert_eq!(old.read_ready(&read), Ok(false));
    println!("old leader n1 at 5000ms: fresh read fence remains pending");
    old.handle_append_entries_response("n2", delayed, 5_001);
    assert_eq!(old.read_ready(&read), Ok(false));
    let result = old.handle_client_query(read);
    assert_eq!(result, Err((catten_graft::types::ERR_READ_BARRIER, Some("n1".to_string()))));
    println!(
        "PASS: delayed term-one reply cannot commit the fresh read fence; stale query rejected"
    );
}
