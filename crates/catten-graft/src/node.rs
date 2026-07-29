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
    configuration::{
        self,
        ConfigurationCommand,
    },
    log_store::{
        LogStore,
        PersistentStateStore,
    },
    membership::ClusterConfiguration,
    state_machine::StateMachine,
    transport::{
        AppendEntriesRpc,
        InstallSnapshotRpc,
        RaftTransport,
        RpcCompletion,
    },
    types::{
        AppendEntriesRequest,
        AppendEntriesResponse,
        ERR_NOT_COMMITTED,
        ERR_NOT_LEADER,
        ERR_READ_BARRIER,
        ERR_TOO_LARGE,
        InstallSnapshotRequest,
        InstallSnapshotResponse,
        LogEntry,
        MAX_COMMAND_BYTES,
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
    pub last_follower_contact_millis: BTreeMap<String, u64>,
    pub applied_command_results: BTreeMap<u64, Vec<u8>>,
    pub snapshot_offsets: BTreeMap<String, u64>,
    pub cluster_configuration: ClusterConfiguration,
    pub snapshot_configuration: ClusterConfiguration,
    pub committed_configurations: BTreeMap<u64, ClusterConfiguration>,
    pub decommissioned: bool,
    pub joining: bool,
    pub pending_join_ids: BTreeSet<String>,
    pub pending_auto_finalize_members: Vec<String>,
    pub pending_auto_finalize_fence_index: u64,
    pub finalize_configuration_pending: bool,
    pub snapshot_min_entries: u64,
    pub snapshot_chunk_bytes: usize,
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

pub struct RaftNodeConfig {
    pub me: Peer,
    pub timeout_millis: u64,
    pub log_store: Box<dyn LogStore>,
    pub persistent_state: Box<dyn PersistentStateStore>,
    pub state_machine: Option<Box<dyn StateMachine>>,
    pub cluster_configuration: ClusterConfiguration,
    pub transport: Arc<dyn RaftTransport>,
    pub current_millis: u64,
    pub snapshot_min_entries: u64,
    pub snapshot_chunk_bytes: usize,
}

impl RaftNode {
    pub fn new(config: RaftNodeConfig) -> Self {
        let RaftNodeConfig {
            me,
            timeout_millis,
            log_store,
            persistent_state,
            state_machine,
            mut cluster_configuration,
            transport,
            current_millis,
            snapshot_min_entries,
            snapshot_chunk_bytes,
        } = config;
        let current_term = persistent_state.current_term();
        let voted_for = persistent_state.voted_for();
        let snapshot_index = log_store.snapshot_index();
        let mut snapshot_configuration = cluster_configuration.clone();
        if snapshot_index > 0 {
            let snapshot_data = log_store.snapshot_data();
            let application_data = if let Some(envelope) =
                crate::snapshot_codec::decode_snapshot_payload(&snapshot_data)
            {
                if !envelope.current_members.is_empty() {
                    snapshot_configuration = ClusterConfiguration::stable(envelope.current_members);
                    if !envelope.next_members.is_empty() {
                        snapshot_configuration =
                            snapshot_configuration.transition_to(envelope.next_members);
                    }
                    cluster_configuration = snapshot_configuration.clone();
                }
                envelope.state_machine_snapshot
            } else {
                snapshot_data
            };
            if let Some(ref machine) = state_machine {
                machine.restore(&application_data);
            }
        }

        let seed =
            me.id.as_bytes().iter().fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(*b as u64));
        let rand_state = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let deadline = current_millis + timeout_millis + ((rand_state >> 33) % 150);
        let decommissioned = !cluster_configuration.contains(&me.id);
        let pending_auto_finalize_members = if cluster_configuration.is_joint_consensus() {
            cluster_configuration.next_members().into_iter().map(|peer| peer.id.clone()).collect()
        } else {
            Vec::new()
        };
        let pending_auto_finalize_fence_index = if cluster_configuration.is_joint_consensus() {
            snapshot_index
        } else {
            0
        };

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
            last_follower_contact_millis: BTreeMap::new(),
            applied_command_results: BTreeMap::new(),
            snapshot_offsets: BTreeMap::new(),
            snapshot_configuration,
            cluster_configuration,
            committed_configurations: BTreeMap::new(),
            decommissioned,
            joining: false,
            pending_join_ids: BTreeSet::new(),
            pending_auto_finalize_members,
            pending_auto_finalize_fence_index,
            finalize_configuration_pending: false,
            snapshot_min_entries,
            snapshot_chunk_bytes,
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
        self.transport.set_current_millis(millis);
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
        if self.joining || !self.cluster_configuration.is_voter(&self.me.id) {
            return false;
        }
        if self.state == NodeState::Leader {
            return false;
        }
        self.current_millis >= self.timeout_at_millis
    }

    pub fn start_election(&mut self, current_millis: u64) {
        self.current_millis = current_millis;
        self.transport.set_current_millis(current_millis);
        if !self.cluster_configuration.is_voter(&self.me.id) {
            self.state = NodeState::Follower;
            self.timeout_at_millis = current_millis + self.election_timeout_millis();
            return;
        }
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

        for peer in self.cluster_configuration.all_members() {
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

        let eligible = req.term >= self.current_term
            && !self.joining
            && self.cluster_configuration.is_voter(&self.me.id)
            && self.cluster_configuration.is_voter(&req.candidate_id);
        if eligible
            && (self.voted_for.is_none() || self.voted_for.as_deref() == Some(&req.candidate_id))
        {
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
            peer_id: self.me.id.clone(),
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

        let is_configured_voter = self.cluster_configuration.is_voter(peer_id);
        if !is_configured_voter {
            return;
        }

        self.granted_votes.insert(peer_id.to_string());
        if self.has_election_majority() {
            self.become_leader(current_millis);
        }
    }

    fn has_election_majority(&self) -> bool {
        self.cluster_configuration.has_joint_majority(&self.granted_votes)
    }

    fn step_down(&mut self, term: u64, current_millis: u64) {
        self.current_term = term;
        self.persistent_state.set_current_term(term);
        self.state = NodeState::Follower;
        self.voted_for = None;
        self.persistent_state.set_voted_for(None);
        self.granted_votes.clear();
        self.last_follower_contact_millis.clear();
        self.known_leader_id = None;
        self.election_sequence_counter = 0;
        self.timeout_at_millis = current_millis + self.election_timeout_millis();
    }

    fn become_leader(&mut self, current_millis: u64) {
        self.state = NodeState::Leader;
        self.known_leader_id = Some(self.me.id.clone());
        self.current_millis = current_millis;
        self.granted_votes.clear();
        self.last_follower_contact_millis.clear();

        let last_index = self.log_store.last_index();
        for peer in self.cluster_configuration.all_members() {
            if peer.id == self.me.id {
                continue;
            }
            self.next_index.insert(peer.id.clone(), last_index + 1);
            self.match_index.insert(peer.id.clone(), 0);
            self.snapshot_offsets.insert(peer.id.clone(), 0);
        }

        // Append a no-op entry for the new term so the leader can commit
        // entries from its own term and advance the commit index.
        self.log_store.append(alloc::vec![crate::types::LogEntry::noop(
            self.current_term,
            self.me.id.clone(),
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
                peer_id: self.me.id.clone(),
                term: self.current_term,
                success: false,
                match_index: 0,
            };
        }

        if req.leader_id == self.me.id
            || (!self.joining && !self.cluster_configuration.is_voter(&req.leader_id))
        {
            return AppendEntriesResponse {
                peer_id: self.me.id.clone(),
                term: self.current_term,
                success: false,
                match_index: self.log_store.last_index(),
            };
        }

        if req.term > self.current_term {
            self.step_down(req.term, current_millis);
        } else if self.state != NodeState::Follower {
            // Valid AppendEntries from the current-term leader: step down
            // without erasing this term's vote. Clearing voted_for here would
            // allow a follower to grant a second vote in the same term.
            self.state = NodeState::Follower;
            self.known_leader_id = None;
            self.granted_votes.clear();
        }

        self.last_heartbeat_millis = current_millis;
        self.timeout_at_millis = current_millis + self.election_timeout_millis();
        self.known_leader_id = Some(req.leader_id);

        let last_index = self.log_store.last_index();
        if req.prev_log_index > last_index {
            return AppendEntriesResponse {
                peer_id: self.me.id.clone(),
                term: self.current_term,
                success: false,
                match_index: last_index,
            };
        }

        if req.prev_log_index > 0 {
            let term_at_prev = self.log_store.term_at(req.prev_log_index);
            if term_at_prev != req.prev_log_term {
                return AppendEntriesResponse {
                    peer_id: self.me.id.clone(),
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
            peer_id: self.me.id.clone(),
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
            self.last_follower_contact_millis.insert(peer_id.to_string(), current_millis);
            self.advance_commit_index();
            self.maybe_auto_finalize_joint_configuration();
        } else {
            if let Some(ni) = self.next_index.get_mut(peer_id)
                && *ni > 1
            {
                *ni -= 1;
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
                peer_id: self.me.id.clone(),
                term: self.current_term,
                success: false,
                last_included_index: req.last_included_index,
                next_offset: 0,
                done: false,
            };
        }

        if req.leader_id == self.me.id
            || (!self.joining && !self.cluster_configuration.is_voter(&req.leader_id))
        {
            return InstallSnapshotResponse {
                peer_id: self.me.id.clone(),
                term: self.current_term,
                success: false,
                last_included_index: self.log_store.snapshot_index(),
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

        // A delayed or retried snapshot must never move application progress
        // backwards or replace a newer snapshot/log. A successful completed
        // response lets the leader resume AppendEntries at the following
        // index.
        if req.last_included_index <= self.commit_index {
            self.pending_snapshot = None;
            return InstallSnapshotResponse {
                peer_id: self.me.id.clone(),
                term: self.current_term,
                success: true,
                last_included_index: req.last_included_index,
                next_offset: req.offset.saturating_add(req.data.len() as u64),
                done: true,
            };
        }

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
                let application_data =
                    if let Some(envelope) = crate::snapshot_codec::decode_snapshot_payload(&data) {
                        if !envelope.current_members.is_empty() {
                            let mut configuration =
                                ClusterConfiguration::stable(envelope.current_members);
                            if !envelope.next_members.is_empty() {
                                configuration = configuration.transition_to(envelope.next_members);
                            }
                            self.snapshot_configuration = configuration.clone();
                            self.cluster_configuration = configuration.clone();
                            self.committed_configurations
                                .retain(|index, _| *index > snap.last_included_index);
                            self.committed_configurations
                                .insert(snap.last_included_index, configuration);
                            self.decommissioned = !self.cluster_configuration.contains(&self.me.id);
                            self.pending_auto_finalize_members = self
                                .cluster_configuration
                                .next_members()
                                .into_iter()
                                .map(|peer| peer.id.clone())
                                .collect();
                            self.pending_auto_finalize_fence_index =
                                if self.cluster_configuration.is_joint_consensus() {
                                    snap.last_included_index
                                } else {
                                    0
                                };
                            self.finalize_configuration_pending = false;
                        }
                        envelope.state_machine_snapshot
                    } else {
                        data
                    };
                if let Some(ref sm) = self.state_machine {
                    sm.restore(&application_data);
                }
            }
            self.pending_snapshot = None;
        }

        InstallSnapshotResponse {
            peer_id: self.me.id.clone(),
            term: self.current_term,
            success: accepted,
            last_included_index: req.last_included_index,
            next_offset,
            done: installed,
        }
    }

    pub fn broadcast_heartbeat(&mut self, current_millis: u64) {
        self.current_millis = current_millis;
        self.transport.set_current_millis(current_millis);

        if self.state != NodeState::Leader {
            return;
        }

        for peer in self.cluster_configuration.all_members() {
            if peer.id == self.me.id {
                continue;
            }

            let ni = *self.next_index.get(&peer.id).unwrap_or(&1);
            let prev_log_index = ni.saturating_sub(1);
            let prev_log_term = if prev_log_index > 0 {
                self.log_store.term_at(prev_log_index)
            } else {
                0
            };

            let snapshot_index = self.log_store.snapshot_index();
            if snapshot_index > 0 && ni <= snapshot_index {
                let snapshot = self.log_store.snapshot_data();
                let offset = self.snapshot_offsets.get(&peer.id).copied().unwrap_or(0) as usize;
                let end = offset.saturating_add(self.snapshot_chunk_bytes).min(snapshot.len());
                if offset <= end {
                    self.transport.send_install_snapshot(InstallSnapshotRpc {
                        peer,
                        term: self.current_term,
                        leader_id: &self.me.id,
                        last_included_index: snapshot_index,
                        last_included_term: self.log_store.snapshot_term(),
                        offset: offset as u64,
                        data: snapshot[offset..end].to_vec(),
                        done: end == snapshot.len(),
                    });
                }
                continue;
            }

            let entries = if ni <= self.log_store.last_index() {
                self.log_store.entries_from(ni)
            } else {
                Vec::new()
            };

            self.transport.send_append_entries(AppendEntriesRpc {
                peer,
                term: self.current_term,
                leader_id: &self.me.id,
                prev_log_index,
                prev_log_term,
                leader_commit: self.commit_index,
                entries,
            });
        }

        self.transport.broadcast_heartbeat_complete();
    }

    /// Drain completed asynchronous transport calls into the consensus state
    /// machine without blocking the shard.
    pub fn poll_transport(&mut self, current_millis: u64) -> usize {
        self.transport.set_current_millis(current_millis);
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
                    sent_next_offset,
                    sent_done,
                } => {
                    if response.term > self.current_term {
                        self.step_down(response.term, current_millis);
                    } else if self.state == NodeState::Leader
                        && response.term == self.current_term
                        && response.success
                    {
                        self.snapshot_offsets.insert(peer_id.clone(), sent_next_offset);
                        if sent_done {
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
        if command.len() > MAX_COMMAND_BYTES {
            return Err(ERR_TOO_LARGE);
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
        let mut candidate_commit = self.commit_index;

        for index in (self.commit_index + 1)..=self.log_store.last_index() {
            if self.log_store.term_at(index) != self.current_term {
                continue;
            }

            // The leader always has its own log entry. Only configured voting
            // followers may contribute the remaining acknowledgements.
            let mut replicated = BTreeSet::new();
            replicated.insert(self.me.id.clone());
            for peer in self.cluster_configuration.all_members() {
                if peer.id != self.me.id
                    && self.match_index.get(&peer.id).copied().unwrap_or(0) >= index
                {
                    replicated.insert(peer.id.clone());
                }
            }
            if self.configuration_at(index).has_joint_majority(&replicated) {
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
                if entry.is_noop() {
                    continue;
                }
                if self.apply_configuration_command(self.last_applied, &entry.data) {
                    continue;
                }
                if let Some(ref sm) = self.state_machine {
                    let result = sm.apply_with_result(entry.term, &entry.data);
                    self.applied_command_results.insert(self.last_applied, result);
                    while self.applied_command_results.len() > 128 {
                        if let Some(oldest) = self.applied_command_results.keys().next().copied() {
                            self.applied_command_results.remove(&oldest);
                        }
                    }
                }
            }
        }
        self.maybe_compact_local_snapshot();
    }

    fn maybe_compact_local_snapshot(&mut self) {
        if self.snapshot_min_entries == 0 || self.state_machine.is_none() || self.commit_index == 0
        {
            return;
        }
        let snapshot_index = self.log_store.snapshot_index();
        if self.commit_index <= snapshot_index
            || self.commit_index - snapshot_index < self.snapshot_min_entries
        {
            return;
        }
        let Some(ref machine) = self.state_machine else {
            return;
        };
        let current_members =
            self.cluster_configuration.current_members().into_iter().cloned().collect::<Vec<_>>();
        let next_members = if self.cluster_configuration.is_joint_consensus() {
            self.cluster_configuration.next_members().into_iter().cloned().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let snapshot = crate::snapshot_codec::wrap_snapshot_payload(
            &current_members,
            &next_members,
            &machine.snapshot(),
        );
        let term = self.log_store.term_at(self.commit_index);
        self.log_store.install_snapshot(self.commit_index, term, snapshot);
        self.snapshot_configuration = self.cluster_configuration.clone();
        self.committed_configurations.retain(|index, _| *index > self.commit_index);
    }

    fn configuration_at(&self, index: u64) -> ClusterConfiguration {
        self.committed_configurations
            .range(..=index)
            .next_back()
            .map(|(_, configuration)| configuration.clone())
            .unwrap_or_else(|| self.snapshot_configuration.clone())
    }

    fn apply_configuration_command(&mut self, index: u64, data: &[u8]) -> bool {
        let Some(command) = configuration::decode(data) else {
            return false;
        };
        if let ConfigurationCommand::Join(ref peer) = command {
            self.pending_join_ids.insert(peer.id.clone());
            return true;
        }
        self.cluster_configuration = match command {
            ConfigurationCommand::Joint(members) => {
                self.pending_auto_finalize_members =
                    members.iter().map(|peer| peer.id.clone()).collect();
                self.pending_auto_finalize_fence_index = index;
                self.finalize_configuration_pending = false;
                self.cluster_configuration.transition_to(members)
            }
            ConfigurationCommand::Finalize => {
                self.pending_auto_finalize_members.clear();
                self.pending_auto_finalize_fence_index = 0;
                self.finalize_configuration_pending = false;
                self.cluster_configuration.finalize_transition()
            }
            ConfigurationCommand::Join(_) => unreachable!(),
        };
        self.committed_configurations.insert(index, self.cluster_configuration.clone());
        self.decommissioned = !self.cluster_configuration.contains(&self.me.id);
        if self.decommissioned {
            self.state = NodeState::Follower;
            self.known_leader_id = None;
        }

        let active_ids: BTreeSet<String> = self
            .cluster_configuration
            .all_members()
            .into_iter()
            .map(|peer| peer.id.clone())
            .collect();
        self.next_index.retain(|id, _| active_ids.contains(id));
        self.match_index.retain(|id, _| active_ids.contains(id));
        self.snapshot_offsets.retain(|id, _| active_ids.contains(id));
        true
    }

    fn maybe_auto_finalize_joint_configuration(&mut self) {
        if self.state != NodeState::Leader
            || !self.cluster_configuration.is_joint_consensus()
            || self.finalize_configuration_pending
            || self.pending_auto_finalize_members.is_empty()
        {
            return;
        }
        let fence = self.pending_auto_finalize_fence_index;
        let caught_up = self.pending_auto_finalize_members.iter().all(|peer_id| {
            peer_id == &self.me.id || self.match_index.get(peer_id).copied().unwrap_or(0) >= fence
        });
        if caught_up && self.submit_finalize_configuration(self.current_millis).is_ok() {
            self.finalize_configuration_pending = true;
        }
    }

    pub fn submit_joint_configuration(
        &mut self,
        members: Vec<Peer>,
        current_millis: u64,
    ) -> Result<u64, i64> {
        let command =
            configuration::encode(&ConfigurationCommand::Joint(members)).ok_or(ERR_TOO_LARGE)?;
        let index = self.submit_command(command, current_millis)?;
        self.maybe_auto_finalize_joint_configuration();
        Ok(index)
    }

    pub fn submit_join(&mut self, peer: Peer, current_millis: u64) -> Result<u64, i64> {
        let command =
            configuration::encode(&ConfigurationCommand::Join(peer)).ok_or(ERR_TOO_LARGE)?;
        self.submit_command(command, current_millis)
    }

    pub fn submit_finalize_configuration(&mut self, current_millis: u64) -> Result<u64, i64> {
        let command =
            configuration::encode(&ConfigurationCommand::Finalize).ok_or(ERR_TOO_LARGE)?;
        self.submit_command(command, current_millis)
    }

    pub fn is_committed(&self, index: u64) -> bool {
        self.commit_index >= index
    }

    pub fn command_result(&self, index: u64) -> Option<&[u8]> {
        self.applied_command_results.get(&index).map(Vec::as_slice)
    }

    pub fn can_serve_linearizable_read(&self) -> bool {
        if self.state != NodeState::Leader {
            return false;
        }
        let mut contacted = BTreeSet::new();
        contacted.insert(self.me.id.clone());
        for (peer_id, last_contact) in &self.last_follower_contact_millis {
            if self.current_millis.saturating_sub(*last_contact) < 750 {
                contacted.insert(peer_id.clone());
            }
        }
        self.cluster_configuration.has_joint_majority(&contacted)
    }

    pub fn handle_client_command(
        &mut self,
        command: Vec<u8>,
        current_millis: u64,
    ) -> Result<Vec<u8>, (i64, Option<String>)> {
        match self.submit_command(command, current_millis) {
            Ok(index) if self.is_committed(index) => {
                Ok(self.command_result(index).unwrap_or_default().to_vec())
            }
            Ok(_) => Err((ERR_NOT_COMMITTED, self.known_leader_id.clone())),
            Err(code) => Err((code, self.known_leader_id.clone())),
        }
    }

    pub fn handle_client_query(&self, query: Vec<u8>) -> Result<Vec<u8>, (i64, Option<String>)> {
        if self.state != NodeState::Leader {
            return Err((ERR_NOT_LEADER, self.known_leader_id.clone()));
        }
        if !self.can_serve_linearizable_read() {
            return Err((ERR_READ_BARRIER, self.known_leader_id.clone()));
        }

        if let Some(ref sm) = self.state_machine
            && let Some(qs) = sm.as_queryable()
        {
            return Ok(qs.query(&query));
        }

        Ok(Vec::new())
    }

    pub fn apply_configuration_change(&mut self, command: &[u8], _current_millis: u64) -> bool {
        // Direct membership mutation is unsafe. Configuration changes must
        // pass through replicated JOINT and FINALIZE log entries.
        let _ = command;
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

    use super::{
        RaftNode,
        RaftNodeConfig,
    };
    use crate::{
        log_store::{
            InMemoryLogStore,
            InMemoryPersistentStateStore,
            LogStore,
        },
        membership::ClusterConfiguration,
        state_machine::StateMachine,
        transport::NoopTransport,
        types::{
            AppendEntriesRequest,
            AppendEntriesResponse,
            InstallSnapshotRequest,
            LogEntry,
            NodeState,
            Peer,
            VoteRequest,
            VoteResponse,
        },
    };

    struct RecordingStateMachine {
        restored: Arc<spin::Mutex<Vec<u8>>>,
    }

    impl StateMachine for RecordingStateMachine {
        fn apply(&self, _term: u64, _command: &[u8]) {}

        fn restore(&self, snapshot_data: &[u8]) {
            *self.restored.lock() = snapshot_data.to_vec();
        }
    }

    struct ApplyingStateMachine {
        applied: Arc<spin::Mutex<Vec<Vec<u8>>>>,
    }

    impl StateMachine for ApplyingStateMachine {
        fn apply(&self, _term: u64, command: &[u8]) {
            self.applied.lock().push(command.to_vec());
        }
    }

    fn node_with_voters(ids: &[&str]) -> RaftNode {
        let peers: Vec<Peer> = ids.iter().map(|id| Peer::voter((*id).to_string(), 0)).collect();
        RaftNode::new(RaftNodeConfig {
            me: peers[0].clone(),
            timeout_millis: 150,
            log_store: Box::new(InMemoryLogStore::new()),
            persistent_state: Box::new(InMemoryPersistentStateStore::new()),
            state_machine: None,
            cluster_configuration: ClusterConfiguration::stable(peers),
            transport: Arc::new(NoopTransport),
            current_millis: 0,
            snapshot_min_entries: 64,
            snapshot_chunk_bytes: 3000,
        })
    }

    #[test]
    fn election_counts_distinct_configured_voters_only() {
        let mut node = node_with_voters(&["n1", "n2", "n3", "n4", "n5"]);
        node.start_election(200);

        let granted = VoteResponse {
            peer_id: "n2".to_string(),
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
    fn learner_does_not_start_an_election() {
        let peers = vec![
            Peer::learner("n1".to_string(), 0),
            Peer::voter("n2".to_string(), 0),
            Peer::voter("n3".to_string(), 0),
        ];
        let mut node = RaftNode::new(RaftNodeConfig {
            me: peers[0].clone(),
            timeout_millis: 150,
            log_store: Box::new(InMemoryLogStore::new()),
            persistent_state: Box::new(InMemoryPersistentStateStore::new()),
            state_machine: None,
            cluster_configuration: ClusterConfiguration::stable(peers),
            transport: Arc::new(NoopTransport),
            current_millis: 0,
            snapshot_min_entries: 64,
            snapshot_chunk_bytes: 3000,
        });

        node.start_election(200);

        assert_eq!(node.state, NodeState::Follower);
        assert_eq!(node.current_term, 0);
        assert_eq!(node.voted_for, None);
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
    fn leader_noop_is_not_applied_as_an_application_command() {
        let applied = Arc::new(spin::Mutex::new(Vec::new()));
        let mut node = RaftNode::new(RaftNodeConfig {
            me: Peer::voter("n1".to_string(), 0),
            timeout_millis: 150,
            log_store: Box::new(InMemoryLogStore::new()),
            persistent_state: Box::new(InMemoryPersistentStateStore::new()),
            state_machine: Some(Box::new(ApplyingStateMachine {
                applied: applied.clone(),
            })),
            cluster_configuration: ClusterConfiguration::stable(vec![Peer::voter(
                "n1".to_string(),
                0,
            )]),
            transport: Arc::new(NoopTransport),
            current_millis: 0,
            snapshot_min_entries: 64,
            snapshot_chunk_bytes: 3000,
        });

        node.start_election(200);
        assert!(applied.lock().is_empty());

        node.submit_command(vec![7], 201).unwrap();
        assert_eq!(&*applied.lock(), &[vec![7]]);
    }

    #[test]
    fn leader_plus_one_follower_commits_in_three_voter_cluster() {
        let mut node = node_with_voters(&["n1", "n2", "n3"]);
        node.start_election(200);
        node.handle_vote_response(
            "n2",
            VoteResponse {
                peer_id: "n2".to_string(),
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
                peer_id: "n2".to_string(),
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
    fn restart_restores_persisted_snapshot_before_marking_it_applied() {
        let log_store = InMemoryLogStore::new();
        log_store.install_snapshot(4, 3, vec![9, 8, 7]);
        let restored = Arc::new(spin::Mutex::new(Vec::new()));

        let node = RaftNode::new(RaftNodeConfig {
            me: Peer::voter("n1".to_string(), 0),
            timeout_millis: 150,
            log_store: Box::new(log_store),
            persistent_state: Box::new(InMemoryPersistentStateStore::new()),
            state_machine: Some(Box::new(RecordingStateMachine {
                restored: restored.clone(),
            })),
            cluster_configuration: ClusterConfiguration::stable(vec![Peer::voter(
                "n1".to_string(),
                0,
            )]),
            transport: Arc::new(NoopTransport),
            current_millis: 0,
            snapshot_min_entries: 64,
            snapshot_chunk_bytes: 3000,
        });

        assert_eq!(&*restored.lock(), &[9, 8, 7]);
        assert_eq!(node.commit_index, 4);
        assert_eq!(node.last_applied, 4);
    }

    #[test]
    fn stale_snapshot_cannot_regress_committed_progress() {
        let mut node = node_with_voters(&["n1", "n2", "n3"]);
        node.log_store.install_snapshot(5, 2, vec![5]);
        node.commit_index = 5;
        node.last_applied = 5;

        let response = node.handle_install_snapshot(
            InstallSnapshotRequest {
                term: node.current_term,
                leader_id: "n2".to_string(),
                last_included_index: 3,
                last_included_term: 1,
                offset: 0,
                data: vec![3],
                done: true,
            },
            201,
        );

        assert!(response.success);
        assert!(response.done);
        assert_eq!(node.log_store.snapshot_index(), 5);
        assert_eq!(node.log_store.snapshot_data(), vec![5]);
        assert_eq!(node.commit_index, 5);
        assert_eq!(node.last_applied, 5);
    }

    #[test]
    fn snapshot_install_retains_a_matching_log_suffix() {
        let store = InMemoryLogStore::new();
        store.append(vec![
            LogEntry::new(1, "n1".to_string(), vec![1]),
            LogEntry::new(2, "n1".to_string(), vec![2]),
            LogEntry::new(3, "n1".to_string(), vec![3]),
        ]);

        store.install_snapshot(2, 2, vec![8]);

        assert_eq!(store.snapshot_index(), 2);
        assert_eq!(store.last_index(), 3);
        assert_eq!(store.term_at(3), 3);
        assert_eq!(store.entry_at(3).unwrap().data, vec![3]);
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

    #[test]
    fn same_term_append_entries_steps_down_without_erasing_vote() {
        let mut node = node_with_voters(&["n1", "n2", "n3"]);
        node.start_election(200);
        let election_term = node.current_term;
        assert_eq!(node.voted_for.as_deref(), Some("n1"));

        let response = node.handle_append_entries(
            AppendEntriesRequest {
                term: election_term,
                leader_id: "n2".to_string(),
                prev_log_index: 0,
                prev_log_term: 0,
                entries: Vec::new(),
                leader_commit: 0,
            },
            201,
        );

        assert!(response.success);
        assert_eq!(node.state, NodeState::Follower);
        assert_eq!(node.voted_for.as_deref(), Some("n1"));
        assert_eq!(node.persistent_state.voted_for().as_deref(), Some("n1"));
    }

    #[test]
    fn vote_and_leader_rpcs_from_non_members_are_rejected() {
        let mut node = node_with_voters(&["n1", "n2", "n3"]);
        let vote = node.handle_vote_request(
            VoteRequest {
                term: 1,
                candidate_id: "outsider".to_string(),
                last_log_index: 0,
                last_log_term: 0,
            },
            200,
        );
        assert!(!vote.vote_granted);

        let append = node.handle_append_entries(
            AppendEntriesRequest {
                term: 2,
                leader_id: "outsider".to_string(),
                prev_log_index: 0,
                prev_log_term: 0,
                entries: Vec::new(),
                leader_commit: 0,
            },
            201,
        );
        assert!(!append.success);
        assert_eq!(node.current_term, 1);
    }

    #[test]
    fn restart_restores_joint_membership_and_roles_from_snapshot() {
        let store = InMemoryLogStore::new();
        let current = vec![Peer::voter("n1".to_string(), 1), Peer::learner("n2".to_string(), 2)];
        let next = vec![Peer::voter("n1".to_string(), 1), Peer::voter("n3".to_string(), 3)];
        let snapshot =
            crate::snapshot_codec::wrap_snapshot_payload(&current, &next, b"application");
        store.install_snapshot(7, 3, snapshot);

        let node = RaftNode::new(RaftNodeConfig {
            me: Peer::voter("n1".to_string(), 1),
            timeout_millis: 150,
            log_store: Box::new(store),
            persistent_state: Box::new(InMemoryPersistentStateStore::new()),
            state_machine: None,
            cluster_configuration: ClusterConfiguration::stable(vec![Peer::voter(
                "obsolete".to_string(),
                99,
            )]),
            transport: Arc::new(NoopTransport),
            current_millis: 0,
            snapshot_min_entries: 64,
            snapshot_chunk_bytes: 3000,
        });

        assert!(node.cluster_configuration.is_joint_consensus());
        assert!(
            node.cluster_configuration
                .current_members()
                .iter()
                .any(|peer| peer.id == "n2" && peer.is_learner())
        );
        assert!(node.cluster_configuration.is_voter("n3"));
        assert_eq!(node.pending_auto_finalize_fence_index, 7);
        assert!(!node.decommissioned);
    }
}
