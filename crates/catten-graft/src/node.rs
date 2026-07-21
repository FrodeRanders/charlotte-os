use alloc::{
    boxed::Box,
    collections::{
        BTreeMap,
        BTreeSet,
    },
    string::{
        String,
        ToString,
    },
    sync::Arc,
    vec::Vec,
};

use crate::{
    log_store::{
        LogStore,
        PersistentStateStore,
    },
    state_machine::StateMachine,
    transport::{
        RaftTransport,
        RpcCompletion,
    },
    types::{
        AppendEntriesRequest,
        AppendEntriesResponse,
        ERR_NOT_LEADER,
        InstallSnapshotRequest,
        InstallSnapshotResponse,
        LogEntry,
        NodeState,
        Peer,
        VoteRequest,
        VoteResponse,
    },
};

pub struct PendingSnapshot {
    pub last_included_index: u64,
    pub last_included_term: u64,
    pub offset: u64,
    pub data: Vec<u8>,
}

pub struct RaftNode {
    pub me: Peer,
    pub state: NodeState,
    pub current_term: u64,
    pub voted_for: Option<String>,
    pub timeout_millis: u64,
    pub last_heartbeat_millis: u64,
    pub timeout_at_millis: u64,
    pub election_sequence_counter: u64,
    pub commit_index: u64,
    pub last_applied: u64,
    pub next_index: BTreeMap<String, u64>,
    pub match_index: BTreeMap<String, u64>,
    pub snapshot_offsets: BTreeMap<String, u64>,
    pub cluster_configuration: Vec<Peer>,
    pub known_leader_id: Option<String>,
    pub pending_snapshot: Option<PendingSnapshot>,
    pub log_store: Box<dyn LogStore>,
    pub persistent_state: Box<dyn PersistentStateStore>,
    pub state_machine: Option<Box<dyn StateMachine>>,
    pub transport: Arc<dyn RaftTransport>,

    current_millis: u64,
    rand_state: u64,
    /// Distinct voter IDs that granted a vote in `current_term`.
    /// This is reset for every election and whenever the node steps down.
    granted_votes: BTreeSet<String>,
}

impl RaftNode {
    pub fn new(
        me: Peer,
        timeout_millis: u64,
        log_store: Box<dyn LogStore>,
        persistent_state: Box<dyn PersistentStateStore>,
        state_machine: Option<Box<dyn StateMachine>>,
        cluster_configuration: Vec<Peer>,
        transport: Arc<dyn RaftTransport>,
        current_millis: u64,
    ) -> Self {
        let current_term = persistent_state.current_term();
        let voted_for = persistent_state.voted_for();
        let snapshot_index = log_store.snapshot_index();

        let seed =
            me.id.as_bytes().iter().fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(*b as u64));
        let rand_state = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let deadline = current_millis + timeout_millis + ((rand_state >> 33) % 150);

        Self {
            me,
            state: NodeState::Follower,
            current_term,
            voted_for,
            timeout_millis,
            last_heartbeat_millis: current_millis,
            timeout_at_millis: deadline,
            election_sequence_counter: 0,
            commit_index: snapshot_index,
            last_applied: snapshot_index,
            next_index: BTreeMap::new(),
            match_index: BTreeMap::new(),
            snapshot_offsets: BTreeMap::new(),
            cluster_configuration,
            known_leader_id: None,
            pending_snapshot: None,
            log_store,
            persistent_state,
            state_machine,
            transport,
            current_millis,
            rand_state,
            granted_votes: BTreeSet::new(),
        }
    }

    pub fn set_millis(&mut self, millis: u64) {
        self.current_millis = millis;
    }

    pub fn millis(&self) -> u64 {
        self.current_millis
    }

    fn random(&mut self) -> u64 {
        self.rand_state =
            self.rand_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.rand_state >> 33
    }

    fn election_timeout_millis(&mut self) -> u64 {
        let base = self.timeout_millis;
        let jitter = self.random() % 150;
        let backoff = (self.timeout_millis / 10) * self.election_sequence_counter;
        base + jitter + backoff
    }

    pub fn check_timeout(&mut self) -> bool {
        if self.state == NodeState::Leader {
            return false;
        }
        self.current_millis >= self.timeout_at_millis
    }

    pub fn start_election(&mut self, current_millis: u64) {
        self.current_millis = current_millis;
        self.state = NodeState::Candidate;
        self.current_term += 1;
        self.persistent_state.set_current_term(self.current_term);
        self.voted_for = Some(self.me.id.clone());
        self.persistent_state.set_voted_for(self.voted_for.clone());
        self.election_sequence_counter += 1;
        self.known_leader_id = None;
        self.granted_votes.clear();
        self.granted_votes.insert(self.me.id.clone());

        self.timeout_at_millis = current_millis + self.election_timeout_millis();

        let last_log_index = self.log_store.last_index();
        let last_log_term = self.log_store.last_term();

        for peer in &self.cluster_configuration {
            if peer.id == self.me.id || !peer.is_voter() {
                continue;
            }
            self.transport.send_vote_request(
                peer,
                self.current_term,
                &self.me.id,
                last_log_index,
                last_log_term,
            );
        }

        // A single-voter cluster already has a majority after the self-vote.
        if self.has_election_majority() {
            self.become_leader(current_millis);
        }
    }

    pub fn handle_vote_request(&mut self, req: VoteRequest, current_millis: u64) -> VoteResponse {
        self.current_millis = current_millis;

        if req.term > self.current_term {
            self.step_down(req.term, current_millis);
        }

        let mut vote_granted = false;

        if req.term < self.current_term {
            vote_granted = false;
        } else if self.voted_for.is_none() || self.voted_for.as_deref() == Some(&req.candidate_id) {
            let last_log_index = self.log_store.last_index();
            let last_log_term = self.log_store.last_term();

            let log_ok = req.last_log_term > last_log_term
                || (req.last_log_term == last_log_term && req.last_log_index >= last_log_index);

            if log_ok {
                self.voted_for = Some(req.candidate_id);
                self.persistent_state.set_voted_for(self.voted_for.clone());
                self.last_heartbeat_millis = current_millis;
                self.timeout_at_millis = current_millis + self.election_timeout_millis();
                vote_granted = true;
            }
        }

        VoteResponse {
            term: self.current_term,
            vote_granted,
        }
    }

    pub fn handle_vote_response(&mut self, peer_id: &str, resp: VoteResponse, current_millis: u64) {
        self.current_millis = current_millis;

        if self.state != NodeState::Candidate {
            return;
        }

        if resp.term > self.current_term {
            self.step_down(resp.term, current_millis);
            return;
        }

        if resp.term != self.current_term || !resp.vote_granted {
            return;
        }

        let is_configured_voter =
            self.cluster_configuration.iter().any(|peer| peer.id == peer_id && peer.is_voter());
        if !is_configured_voter {
            return;
        }

        self.granted_votes.insert(peer_id.to_string());
        if self.has_election_majority() {
            self.become_leader(current_millis);
        }
    }

    fn has_election_majority(&self) -> bool {
        let voter_count = self.cluster_configuration.iter().filter(|peer| peer.is_voter()).count();
        if voter_count == 0 {
            return false;
        }
        let granted = self
            .cluster_configuration
            .iter()
            .filter(|peer| peer.is_voter() && self.granted_votes.contains(&peer.id))
            .count();
        granted >= voter_count / 2 + 1
    }

    fn step_down(&mut self, term: u64, current_millis: u64) {
        self.current_term = term;
        self.persistent_state.set_current_term(term);
        self.state = NodeState::Follower;
        self.voted_for = None;
        self.persistent_state.set_voted_for(None);
        self.granted_votes.clear();
        self.known_leader_id = None;
        self.election_sequence_counter = 0;
        self.timeout_at_millis = current_millis + self.election_timeout_millis();
    }

    fn become_leader(&mut self, current_millis: u64) {
        self.state = NodeState::Leader;
        self.known_leader_id = Some(self.me.id.clone());
        self.current_millis = current_millis;
        self.granted_votes.clear();

        let last_index = self.log_store.last_index();
        for peer in &self.cluster_configuration {
            if peer.id == self.me.id {
                continue;
            }
            self.next_index.insert(peer.id.clone(), last_index + 1);
            self.match_index.insert(peer.id.clone(), 0);
            self.snapshot_offsets.insert(peer.id.clone(), 0);
        }

        // Append a no-op entry for the new term so the leader can commit
        // entries from its own term and advance the commit index.
        self.log_store.append(alloc::vec![crate::types::LogEntry::new(
            self.current_term,
            self.me.id.clone(),
            alloc::vec![0u8],
        )]);
        self.advance_commit_index();
    }

    pub fn handle_append_entries(
        &mut self,
        req: AppendEntriesRequest,
        current_millis: u64,
    ) -> AppendEntriesResponse {
        self.current_millis = current_millis;

        if req.term < self.current_term {
            return AppendEntriesResponse {
                term: self.current_term,
                success: false,
                match_index: 0,
            };
        }

        if req.term > self.current_term {
            self.step_down(req.term, current_millis);
        } else if self.state != NodeState::Follower {
            // Valid AppendEntries from the current-term leader: step down
            // from Candidate to Follower, clearing election state.
            self.state = NodeState::Follower;
            self.known_leader_id = None;
            self.granted_votes.clear();
            self.voted_for = None;
            self.persistent_state.set_voted_for(None);
        }

        self.last_heartbeat_millis = current_millis;
        self.timeout_at_millis = current_millis + self.election_timeout_millis();
        self.known_leader_id = Some(req.leader_id);

        let last_index = self.log_store.last_index();
        if req.prev_log_index > last_index {
            return AppendEntriesResponse {
                term: self.current_term,
                success: false,
                match_index: last_index,
            };
        }

        if req.prev_log_index > 0 {
            let term_at_prev = self.log_store.term_at(req.prev_log_index);
            if term_at_prev != req.prev_log_term {
                return AppendEntriesResponse {
                    term: self.current_term,
                    success: false,
                    match_index: req.prev_log_index,
                };
            }
        }

        let mut _conflict_idx: u64 = 0;
        for (i, entry) in req.entries.iter().enumerate() {
            let idx = req.prev_log_index + 1 + i as u64;
            if idx <= self.log_store.last_index() {
                let existing_term = self.log_store.term_at(idx);
                if existing_term != entry.term {
                    self.log_store.truncate_from(idx);
                    self.log_store.append(req.entries[i..].to_vec());
                    _conflict_idx = idx;
                    break;
                }
            } else {
                self.log_store.append(req.entries[i..].to_vec());
                break;
            }
        }

        if req.leader_commit > self.commit_index {
            let last_new_index = req.prev_log_index + req.entries.len() as u64;
            let new_commit = if req.leader_commit < last_new_index {
                req.leader_commit
            } else {
                last_new_index
            };
            if new_commit > self.commit_index {
                self.commit_index = new_commit;
                self.apply_committed();
            }
        }

        AppendEntriesResponse {
            term: self.current_term,
            success: true,
            match_index: req.prev_log_index + req.entries.len() as u64,
        }
    }

    pub fn handle_append_entries_response(
        &mut self,
        peer_id: &str,
        resp: AppendEntriesResponse,
        current_millis: u64,
    ) {
        self.current_millis = current_millis;

        if self.state != NodeState::Leader {
            return;
        }

        if resp.term > self.current_term {
            self.step_down(resp.term, current_millis);
            return;
        }

        if resp.success {
            let new_match = resp.match_index;
            self.match_index.insert(peer_id.to_string(), new_match);
            self.next_index.insert(peer_id.to_string(), new_match + 1);
            self.advance_commit_index();
        } else {
            if let Some(ni) = self.next_index.get_mut(peer_id) {
                if *ni > 1 {
                    *ni -= 1;
                }
            }
        }
    }

    pub fn handle_install_snapshot(
        &mut self,
        req: InstallSnapshotRequest,
        current_millis: u64,
    ) -> InstallSnapshotResponse {
        self.current_millis = current_millis;

        if req.term < self.current_term {
            return InstallSnapshotResponse {
                term: self.current_term,
                success: false,
                last_included_index: req.last_included_index,
                next_offset: 0,
                done: false,
            };
        }

        if req.term > self.current_term {
            self.step_down(req.term, current_millis);
        } else if self.state != NodeState::Follower {
            // A valid snapshot from the current-term leader establishes that
            // this node is not the leader for the term. Keep voted_for for the
            // current term so a follower cannot vote twice.
            self.state = NodeState::Follower;
            self.known_leader_id = None;
            self.granted_votes.clear();
        }

        self.last_heartbeat_millis = current_millis;
        self.timeout_at_millis = current_millis + self.election_timeout_millis();
        self.known_leader_id = Some(req.leader_id);

        if req.offset == 0 {
            self.pending_snapshot = Some(PendingSnapshot {
                last_included_index: req.last_included_index,
                last_included_term: req.last_included_term,
                offset: 0,
                data: Vec::new(),
            });
        }

        let mut accepted = false;
        let mut next_offset = 0;
        if let Some(ref mut snap) = self.pending_snapshot {
            next_offset = snap.offset;
            if req.last_included_index == snap.last_included_index
                && req.last_included_term == snap.last_included_term
                && req.offset == snap.offset
            {
                snap.data.extend_from_slice(&req.data);
                snap.offset += req.data.len() as u64;
                next_offset = snap.offset;
                accepted = true;
            }
        }

        let installed = accepted && req.done;
        if installed {
            if let Some(ref snap) = self.pending_snapshot {
                let data = snap.data.clone();
                self.log_store.install_snapshot(
                    snap.last_included_index,
                    snap.last_included_term,
                    data.clone(),
                );
                self.commit_index = snap.last_included_index;
                self.last_applied = snap.last_included_index;
                if let Some(ref sm) = self.state_machine {
                    sm.restore(&data);
                }
            }
            self.pending_snapshot = None;
        }

        InstallSnapshotResponse {
            term: self.current_term,
            success: accepted,
            last_included_index: req.last_included_index,
            next_offset,
            done: installed,
        }
    }

    pub fn broadcast_heartbeat(&mut self, current_millis: u64) {
        self.current_millis = current_millis;

        if self.state != NodeState::Leader {
            return;
        }

        for peer in &self.cluster_configuration {
            if peer.id == self.me.id {
                continue;
            }

            let ni = *self.next_index.get(&peer.id).unwrap_or(&1);
            let prev_log_index = if ni > 1 {
                ni - 1
            } else {
                0
            };
            let prev_log_term = if prev_log_index > 0 {
                self.log_store.term_at(prev_log_index)
            } else {
                0
            };

            let snapshot_index = self.log_store.snapshot_index();
            if snapshot_index > 0 && ni <= snapshot_index {
                const SNAPSHOT_CHUNK: usize = 3000;
                let snapshot = self.log_store.snapshot_data();
                let offset = self.snapshot_offsets.get(&peer.id).copied().unwrap_or(0) as usize;
                let end = (offset + SNAPSHOT_CHUNK).min(snapshot.len());
                if offset <= end {
                    self.transport.send_install_snapshot(
                        peer,
                        self.current_term,
                        &self.me.id,
                        snapshot_index,
                        self.log_store.snapshot_term(),
                        offset as u64,
                        snapshot[offset..end].to_vec(),
                        end == snapshot.len(),
                    );
                }
                continue;
            }

            let entries = if ni <= self.log_store.last_index() {
                self.log_store.entries_from(ni)
            } else {
                Vec::new()
            };

            self.transport.send_append_entries(
                peer,
                self.current_term,
                &self.me.id,
                prev_log_index,
                prev_log_term,
                self.commit_index,
                entries,
            );
        }

        self.transport.broadcast_heartbeat_complete();
    }

    /// Drain completed asynchronous transport calls into the consensus state
    /// machine without blocking the shard.
    pub fn poll_transport(&mut self, current_millis: u64) -> usize {
        let completions = self.transport.poll_completions();
        let count = completions.len();
        for completion in completions {
            match completion {
                RpcCompletion::Vote {
                    peer_id,
                    response,
                } => {
                    self.handle_vote_response(&peer_id, response, current_millis);
                }
                RpcCompletion::AppendEntries {
                    peer_id,
                    response,
                } => {
                    self.handle_append_entries_response(&peer_id, response, current_millis);
                }
                RpcCompletion::InstallSnapshot {
                    peer_id,
                    response,
                } => {
                    if response.term > self.current_term {
                        self.step_down(response.term, current_millis);
                    } else if self.state == NodeState::Leader
                        && response.term == self.current_term
                        && response.success
                    {
                        self.snapshot_offsets.insert(peer_id.clone(), response.next_offset);
                        if response.done {
                            self.match_index.insert(peer_id.clone(), response.last_included_index);
                            self.next_index
                                .insert(peer_id.clone(), response.last_included_index + 1);
                            self.snapshot_offsets.insert(peer_id, 0);
                            self.advance_commit_index();
                        }
                    }
                }
            }
        }
        count
    }

    pub fn submit_command(&mut self, command: Vec<u8>, current_millis: u64) -> Result<u64, i64> {
        self.current_millis = current_millis;

        if self.state != NodeState::Leader {
            return Err(ERR_NOT_LEADER);
        }

        let entry = LogEntry::new(self.current_term, self.me.id.clone(), command);
        self.log_store.append(alloc::vec![entry]);

        let index = self.log_store.last_index();
        // The leader's own log is authoritative locally and is counted
        // explicitly by `advance_commit_index`; match_index remains
        // follower-only, like the upstream implementation.
        self.advance_commit_index();

        Ok(index)
    }

    fn advance_commit_index(&mut self) {
        let voter_count = self.cluster_configuration.iter().filter(|peer| peer.is_voter()).count();
        if voter_count == 0 {
            return;
        }
        let quorum = voter_count / 2 + 1;
        let mut candidate_commit = self.commit_index;

        for index in (self.commit_index + 1)..=self.log_store.last_index() {
            if self.log_store.term_at(index) != self.current_term {
                continue;
            }

            // The leader always has its own log entry. Only configured voting
            // followers may contribute the remaining acknowledgements.
            let replicated = 1 + self
                .cluster_configuration
                .iter()
                .filter(|peer| peer.id != self.me.id && peer.is_voter())
                .filter(|peer| self.match_index.get(&peer.id).copied().unwrap_or(0) >= index)
                .count();
            if replicated >= quorum {
                candidate_commit = index;
            }
        }

        if candidate_commit > self.commit_index {
            self.commit_index = candidate_commit;
            self.apply_committed();
        }
    }

    fn apply_committed(&mut self) {
        while self.last_applied < self.commit_index {
            self.last_applied += 1;
            if let Some(entry) = self.log_store.entry_at(self.last_applied) {
                if !entry.is_noop() {
                    if let Some(ref sm) = self.state_machine {
                        sm.apply(entry.term, &entry.data);
                    }
                }
            }
        }
    }

    pub fn handle_client_command(
        &mut self,
        command: Vec<u8>,
        current_millis: u64,
    ) -> Result<Vec<u8>, (i64, Option<String>)> {
        match self.submit_command(command, current_millis) {
            Ok(_index) => Ok(Vec::new()),
            Err(_) => Err((ERR_NOT_LEADER, self.known_leader_id.clone())),
        }
    }

    pub fn handle_client_query(&self, query: Vec<u8>) -> Result<Vec<u8>, (i64, Option<String>)> {
        if self.state != NodeState::Leader {
            return Err((ERR_NOT_LEADER, self.known_leader_id.clone()));
        }

        if let Some(ref sm) = self.state_machine {
            if let Some(qs) = sm.as_queryable() {
                return Ok(qs.query(&query));
            }
        }

        Ok(Vec::new())
    }

    pub fn apply_configuration_change(&mut self, command: &[u8], _current_millis: u64) -> bool {
        use alloc::string::String;

        let cmd_str = core::str::from_utf8(command).unwrap_or("");
        let parts: Vec<&str> = cmd_str.splitn(3, ':').collect();

        match parts.first() {
            Some(&"JOIN") => {
                if parts.len() >= 3 {
                    let peer_id = String::from(parts[1]);
                    let name_bytes = parts[2].as_bytes();
                    let mut packed = [0u8; 8];
                    let len = name_bytes.len().min(8);
                    packed[..len].copy_from_slice(&name_bytes[..len]);
                    let service_name = u64::from_le_bytes(packed);

                    let already = self.cluster_configuration.iter().any(|p| p.id == peer_id);
                    if !already {
                        self.cluster_configuration.push(Peer::voter(peer_id, service_name));
                    }
                    return true;
                }
            }
            Some(&"REMOVE") => {
                if parts.len() >= 2 {
                    let peer_id = parts[1];
                    self.cluster_configuration.retain(|p| p.id != peer_id);
                    self.next_index.remove(peer_id);
                    self.match_index.remove(peer_id);
                    return true;
                }
            }
            _ => {}
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use alloc::{
        boxed::Box,
        string::ToString,
        sync::Arc,
        vec,
        vec::Vec,
    };

    use super::RaftNode;
    use crate::{
        log_store::{
            InMemoryLogStore,
            InMemoryPersistentStateStore,
        },
        transport::NoopTransport,
        types::{
            AppendEntriesResponse,
            InstallSnapshotRequest,
            NodeState,
            Peer,
            VoteResponse,
        },
    };

    fn node_with_voters(ids: &[&str]) -> RaftNode {
        let peers: Vec<Peer> = ids.iter().map(|id| Peer::voter((*id).to_string(), 0)).collect();
        RaftNode::new(
            peers[0].clone(),
            150,
            Box::new(InMemoryLogStore::new()),
            Box::new(InMemoryPersistentStateStore::new()),
            None,
            peers,
            Arc::new(NoopTransport),
            0,
        )
    }

    #[test]
    fn election_counts_distinct_configured_voters_only() {
        let mut node = node_with_voters(&["n1", "n2", "n3", "n4", "n5"]);
        node.start_election(200);

        let granted = VoteResponse {
            term: node.current_term,
            vote_granted: true,
        };
        node.handle_vote_response("n2", granted.clone(), 201);
        assert_eq!(node.state, NodeState::Candidate);

        // Duplicate and unknown responses must not manufacture a quorum.
        node.handle_vote_response("n2", granted.clone(), 202);
        node.handle_vote_response("not-a-member", granted.clone(), 203);
        assert_eq!(node.state, NodeState::Candidate);

        node.handle_vote_response("n3", granted, 204);
        assert_eq!(node.state, NodeState::Leader);
    }

    #[test]
    fn single_voter_elects_and_commits_with_its_self_vote() {
        let mut node = node_with_voters(&["n1"]);
        node.start_election(200);
        assert_eq!(node.state, NodeState::Leader);

        // Index 1 is the no-op entry appended by become_leader.
        let index = node.submit_command(vec![1, 2, 3], 201).unwrap();
        assert_eq!(index, 2);
        assert_eq!(node.commit_index, 2);
        assert_eq!(node.last_applied, 2);
    }

    #[test]
    fn leader_plus_one_follower_commits_in_three_voter_cluster() {
        let mut node = node_with_voters(&["n1", "n2", "n3"]);
        node.start_election(200);
        node.handle_vote_response(
            "n2",
            VoteResponse {
                term: node.current_term,
                vote_granted: true,
            },
            201,
        );
        assert_eq!(node.state, NodeState::Leader);

        let index = node.submit_command(vec![7], 202).unwrap();
        assert_eq!(node.commit_index, 0);
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                term: node.current_term,
                success: true,
                match_index: index,
            },
            203,
        );
        assert_eq!(node.commit_index, index);
    }

    #[test]
    fn higher_term_snapshot_persists_term_and_steps_down() {
        let mut node = node_with_voters(&["n1", "n2", "n3"]);
        node.start_election(200);
        assert_eq!(node.state, NodeState::Candidate);

        let response = node.handle_install_snapshot(
            InstallSnapshotRequest {
                term: 5,
                leader_id: "n2".to_string(),
                last_included_index: 4,
                last_included_term: 4,
                offset: 0,
                data: vec![9],
                done: true,
            },
            201,
        );

        assert_eq!(response.term, 5);
        assert_eq!(node.current_term, 5);
        assert_eq!(node.state, NodeState::Follower);
        assert_eq!(node.voted_for, None);
        assert_eq!(node.persistent_state.current_term(), 5);
        assert_eq!(node.persistent_state.voted_for(), None);
        assert_eq!(node.known_leader_id.as_deref(), Some("n2"));
    }

    #[test]
    fn same_term_snapshot_steps_down_without_erasing_vote() {
        let mut node = node_with_voters(&["n1", "n2", "n3"]);
        node.start_election(200);
        let election_term = node.current_term;
        assert_eq!(node.voted_for.as_deref(), Some("n1"));

        node.handle_install_snapshot(
            InstallSnapshotRequest {
                term: election_term,
                leader_id: "n2".to_string(),
                last_included_index: 0,
                last_included_term: 0,
                offset: 0,
                data: Vec::new(),
                done: true,
            },
            201,
        );

        assert_eq!(node.state, NodeState::Follower);
        assert_eq!(node.voted_for.as_deref(), Some("n1"));
        assert_eq!(node.persistent_state.voted_for().as_deref(), Some("n1"));
    }
}
