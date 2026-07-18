use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use crate::log_store::{LogStore, PersistentStateStore};
use crate::state_machine::StateMachine;
use crate::transport::RaftTransport;
use crate::types::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest,
    InstallSnapshotResponse, LogEntry, NodeState, Peer,
    VoteRequest, VoteResponse, ERR_NOT_LEADER,
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
    pub cluster_configuration: Vec<Peer>,
    pub known_leader_id: Option<String>,
    pub pending_snapshot: Option<PendingSnapshot>,
    pub log_store: Box<dyn LogStore>,
    pub persistent_state: Box<dyn PersistentStateStore>,
    pub state_machine: Option<Box<dyn StateMachine>>,
    pub transport: Box<dyn RaftTransport>,

    current_millis: u64,
    rand_state: u64,
}

impl RaftNode {
    pub fn new(
        me: Peer,
        timeout_millis: u64,
        log_store: Box<dyn LogStore>,
        persistent_state: Box<dyn PersistentStateStore>,
        state_machine: Option<Box<dyn StateMachine>>,
        cluster_configuration: Vec<Peer>,
        transport: Box<dyn RaftTransport>,
        current_millis: u64,
    ) -> Self {
        let current_term = persistent_state.current_term();
        let voted_for = persistent_state.voted_for();
        let snapshot_index = log_store.snapshot_index();

        let rand_state = me.id.as_bytes().iter().fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(*b as u64));
        let deadline = current_millis + timeout_millis + (me.id.as_bytes().len() as u64 * 17 % 150);

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
            cluster_configuration,
            known_leader_id: None,
            pending_snapshot: None,
            log_store,
            persistent_state,
            state_machine,
            transport,
            current_millis,
            rand_state,
        }
    }

    pub fn set_millis(&mut self, millis: u64) {
        self.current_millis = millis;
    }

    pub fn millis(&self) -> u64 {
        self.current_millis
    }

    fn random(&mut self) -> u64 {
        self.rand_state = self.rand_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
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
    }

    pub fn handle_vote_request(&mut self, req: VoteRequest, current_millis: u64) -> VoteResponse {
        self.current_millis = current_millis;

        if req.term > self.current_term {
            self.current_term = req.term;
            self.persistent_state.set_current_term(self.current_term);
            self.state = NodeState::Follower;
            self.voted_for = None;
            self.persistent_state.set_voted_for(None);
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

    pub fn handle_vote_response(&mut self, _peer_id: &str, resp: VoteResponse, current_millis: u64) {
        self.current_millis = current_millis;

        if self.state != NodeState::Candidate {
            return;
        }

        if resp.term > self.current_term {
            self.step_down(resp.term, current_millis);
            return;
        }

        if resp.term == self.current_term && resp.vote_granted {
            let voters: Vec<&Peer> = self
                .cluster_configuration
                .iter()
                .filter(|p| p.is_voter())
                .collect();
            let mut granted: usize = 1;
            for p in &voters {
                if p.id == self.me.id {
                    continue;
                }
                granted += 1;
            }

            let majority = (voters.len() / 2) + 1;
            if granted >= majority {
                self.become_leader(current_millis);
            }
        }
    }

    fn step_down(&mut self, term: u64, current_millis: u64) {
        self.current_term = term;
        self.persistent_state.set_current_term(term);
        self.state = NodeState::Follower;
        self.voted_for = None;
        self.persistent_state.set_voted_for(None);
        self.timeout_at_millis = current_millis + self.election_timeout_millis();
    }

    fn become_leader(&mut self, current_millis: u64) {
        self.state = NodeState::Leader;
        self.known_leader_id = Some(self.me.id.clone());
        self.current_millis = current_millis;

        let last_index = self.log_store.last_index();
        for peer in &self.cluster_configuration {
            if peer.id == self.me.id {
                continue;
            }
            self.next_index.insert(peer.id.clone(), last_index + 1);
            self.match_index.insert(peer.id.clone(), 0);
        }
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
            self.current_term = req.term;
            self.persistent_state.set_current_term(self.current_term);
            self.state = NodeState::Follower;
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
            };
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

        if let Some(ref mut snap) = self.pending_snapshot {
            if req.offset == snap.offset {
                snap.data.extend_from_slice(&req.data);
                snap.offset += req.data.len() as u64;
            }
        }

        if req.done {
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
            let prev_log_index = if ni > 1 { ni - 1 } else { 0 };
            let prev_log_term = if prev_log_index > 0 {
                self.log_store.term_at(prev_log_index)
            } else {
                0
            };

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

    pub fn submit_command(&mut self, command: Vec<u8>, current_millis: u64) -> Result<u64, i64> {
        self.current_millis = current_millis;

        if self.state != NodeState::Leader {
            return Err(ERR_NOT_LEADER);
        }

        let entry = LogEntry::new(self.current_term, self.me.id.clone(), command);
        self.log_store.append(alloc::vec![entry]);

        let index = self.log_store.last_index();
        if let Some(ni) = self.next_index.get_mut(&self.me.id) {
            *ni = index + 1;
        }
        if let Some(mi) = self.match_index.get_mut(&self.me.id) {
            *mi = index;
        }

        Ok(index)
    }

    fn advance_commit_index(&mut self) {
        let mut match_indices: Vec<u64> = self.match_index.values().copied().collect();
        match_indices.sort_unstable_by(|a, b| b.cmp(a));

        let voters: Vec<&Peer> = self
            .cluster_configuration
            .iter()
            .filter(|p| p.is_voter())
            .collect();
        let quorum = (voters.len() / 2) + 1;

        if match_indices.len() >= quorum {
            let candidate = match_indices[quorum - 1];
            if candidate > self.commit_index && self.log_store.term_at(candidate) == self.current_term {
                self.commit_index = candidate;
                self.apply_committed();
            }
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
