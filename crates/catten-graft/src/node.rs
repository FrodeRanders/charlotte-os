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
        JoinAdmission,
        LogStore,
        PersistentStateStore,
    },
    membership::ClusterConfiguration,
    state_machine::{
        SnapshotError,
        StateMachine,
    },
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

struct PendingSnapshot {
    leader_id: String,
    leader_term: u64,
    last_included_index: u64,
    last_included_term: u64,
    offset: u64,
    data: Vec<u8>,
}

/// One query admitted before its fresh log fence. Consuming this owner queries
/// at most once; dropping it abandons only the read, not the replicated no-op.
#[must_use]
pub struct PendingRead {
    domain: Arc<()>,
    term: u64,
    index: u64,
    membership_epoch: u64,
    query: Vec<u8>,
}

pub struct RaftNode {
    pub me: Peer,
    pub state: NodeState,
    pub current_term: u64,
    pub voted_for: Option<String>,
    pub timeout_millis: u64,
    pub last_heartbeat_millis: u64,
    /// Last time this follower successfully matched and applied an
    /// AppendEntries request from its recognized leader. Unlike
    /// `last_heartbeat_millis`, rejected replication does not refresh this
    /// bounded-read witness.
    last_successful_leader_sync_millis: Option<u64>,
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
    /// Exact committed configuration index, persisted in membership snapshot
    /// envelopes independently of ordinary state-machine log compaction.
    pub membership_epoch: u64,
    pub decommissioned: bool,
    pub joining: bool,
    /// The one leader this node explicitly elected to join. Joining does not
    /// authorize arbitrary peers on the segment to replace our log.
    pub joining_from: Option<String>,
    /// Peers admitted by committed JOIN entries, keyed by peer id and mapped
    /// to their Peer spec plus the fence: the log index of the committed
    /// JOIN. The leader replicates to these peers and promotes them to the
    /// joint configuration only once they have caught up to the fence.
    pub pending_joiners: BTreeMap<String, (Peer, u64)>,
    pub pending_auto_finalize_members: Vec<String>,
    pub pending_auto_finalize_fence_index: u64,
    pub finalize_configuration_pending: bool,
    pub snapshot_min_entries: u64,
    pub snapshot_chunk_bytes: usize,
    pub known_leader_id: Option<String>,
    pending_snapshot: Option<PendingSnapshot>,
    pub log_store: Box<dyn LogStore>,
    pub persistent_state: Box<dyn PersistentStateStore>,
    pub state_machine: Option<Box<dyn StateMachine>>,
    pub transport: Arc<dyn RaftTransport>,

    current_millis: u64,
    rand_state: u64,
    /// Distinct voter IDs that granted a vote in `current_term`.
    /// This is reset for every election and whenever the node steps down.
    granted_votes: BTreeSet<String>,
    // Lazy, fallible identity retained by pending reads across node retirement.
    // Domain replacement invalidates every earlier read even if indices match.
    read_domain: Option<Arc<()>>,
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
    pub fn new(config: RaftNodeConfig) -> Result<Self, SnapshotError> {
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
        let join_admission = persistent_state.join_admission();
        let snapshot_index = log_store.snapshot_index();
        let mut snapshot_configuration = cluster_configuration.clone();
        let mut membership_epoch = 0;
        let mut restored_membership_snapshot = false;
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
                    restored_membership_snapshot = true;
                    membership_epoch = if envelope.membership_epoch == 0 {
                        snapshot_index
                    } else {
                        envelope.membership_epoch
                    };
                }
                envelope.state_machine_snapshot
            } else {
                snapshot_data
            };
            if let Some(ref machine) = state_machine {
                machine.restore(&application_data)?;
            }
        }

        let seed =
            me.id.as_bytes().iter().fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(*b as u64));
        let rand_state = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let deadline = current_millis + timeout_millis + ((rand_state >> 33) % 150);
        let joining_from = join_admission.and_then(|admission| {
            let admission_completed = restored_membership_snapshot
                && snapshot_index > admission.snapshot_index
                && cluster_configuration.contains(&me.id);
            if admission_completed {
                persistent_state.set_join_admission(None);
                None
            } else {
                Some(admission.anchor_id)
            }
        });
        let joining = joining_from.is_some();
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

        Ok(Self {
            me,
            state: NodeState::Follower,
            current_term,
            voted_for,
            timeout_millis,
            last_heartbeat_millis: current_millis,
            last_successful_leader_sync_millis: None,
            timeout_at_millis: if joining {
                u64::MAX
            } else {
                deadline
            },
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
            membership_epoch,
            decommissioned,
            joining,
            joining_from: joining_from.clone(),
            pending_joiners: BTreeMap::new(),
            pending_auto_finalize_members,
            pending_auto_finalize_fence_index,
            finalize_configuration_pending: false,
            snapshot_min_entries,
            snapshot_chunk_bytes,
            known_leader_id: joining_from,
            pending_snapshot: None,
            log_store,
            persistent_state,
            state_machine,
            transport,
            current_millis,
            rand_state,
            granted_votes: BTreeSet::new(),
            read_domain: None,
        })
    }

    pub fn set_millis(&mut self, millis: u64) {
        self.current_millis = millis;
        self.transport.set_current_millis(millis);
    }

    pub fn millis(&self) -> u64 {
        self.current_millis
    }

    /// Raft index of the newest committed membership snapshot/change.
    ///
    /// This is a stable, cluster-wide generation suitable for consumers that
    /// materialize membership outside the consensus reactor. Ordinary log
    /// compaction preserves it through the snapshot index.
    pub fn membership_epoch(&self) -> u64 {
        self.membership_epoch
    }

    /// Enter the non-voting admission state for a specific cluster anchor.
    /// The node stops campaigning and accepts replication only from that
    /// anchor until a committed configuration contains this node.
    pub fn begin_joining(&mut self, anchor_id: String, current_millis: u64) {
        // Persist the admission fence before disabling elections or accepting
        // the anchor's log. A crash at any later instruction therefore
        // restores the fail-closed joining posture. A fresh node may already
        // have elected itself and committed a standalone term-1 history. That
        // history belongs to a different consensus domain even when its term
        // and indexes happen to equal the anchor's; retaining it would violate
        // Raft's log-matching premise. Clear it only after the durable fence is
        // present, then let the selected anchor install the authoritative log.
        let standalone_snapshot_index = self.log_store.snapshot_index();
        self.persistent_state.set_join_admission(Some(JoinAdmission {
            anchor_id: anchor_id.clone(),
            snapshot_index: standalone_snapshot_index,
        }));
        self.transport.reset();
        self.read_domain = None;
        self.log_store.reset();
        if let Some(ref machine) = self.state_machine {
            machine.reset();
        }
        self.commit_index = 0;
        self.last_applied = 0;
        self.membership_epoch = 0;
        self.next_index.clear();
        self.match_index.clear();
        self.last_follower_contact_millis.clear();
        self.snapshot_offsets.clear();
        self.committed_configurations.clear();
        self.applied_command_results.clear();
        self.pending_joiners.clear();
        self.pending_auto_finalize_members.clear();
        self.pending_auto_finalize_fence_index = 0;
        self.finalize_configuration_pending = false;
        self.pending_snapshot = None;
        self.voted_for = None;
        self.persistent_state.set_voted_for(None);
        self.current_millis = current_millis;
        self.last_heartbeat_millis = current_millis;
        self.last_successful_leader_sync_millis = None;
        self.election_sequence_counter = 0;
        self.state = NodeState::Follower;
        self.joining = true;
        self.joining_from = Some(anchor_id.clone());
        self.known_leader_id = Some(anchor_id);
        self.granted_votes.clear();
        self.timeout_at_millis = u64::MAX;
    }

    fn accepts_joining_leader(&self, leader_id: &str) -> bool {
        self.joining && self.joining_from.as_deref() == Some(leader_id)
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
        if self.joining || !self.cluster_configuration.is_voter(&self.me.id) {
            self.state = NodeState::Follower;
            self.timeout_at_millis = current_millis + self.election_timeout_millis();
            return;
        }
        self.state = NodeState::Candidate;
        self.pending_snapshot = None;
        self.current_term += 1;
        self.persistent_state.set_current_term(self.current_term);
        self.voted_for = Some(self.me.id.clone());
        self.persistent_state.set_voted_for(self.voted_for.clone());
        self.election_sequence_counter += 1;
        self.known_leader_id = None;
        self.last_successful_leader_sync_millis = None;
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
        // A new election term cannot continue a former leader's byte stream.
        self.pending_snapshot = None;
        self.current_term = term;
        self.persistent_state.set_current_term(term);
        self.state = NodeState::Follower;
        self.voted_for = None;
        self.persistent_state.set_voted_for(None);
        self.granted_votes.clear();
        self.last_follower_contact_millis.clear();
        self.known_leader_id = None;
        self.last_successful_leader_sync_millis = None;
        self.election_sequence_counter = 0;
        self.timeout_at_millis = current_millis + self.election_timeout_millis();
    }

    fn become_leader(&mut self, current_millis: u64) {
        self.state = NodeState::Leader;
        self.known_leader_id = Some(self.me.id.clone());
        self.last_successful_leader_sync_millis = None;
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
        mut req: AppendEntriesRequest,
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
            || (!self.cluster_configuration.is_voter(&req.leader_id)
                && !self.accepts_joining_leader(&req.leader_id))
            || (req.term == self.current_term
                && self.state == NodeState::Follower
                && self.known_leader_id.as_deref().is_some_and(|leader| leader != req.leader_id))
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
            self.last_successful_leader_sync_millis = None;
            self.granted_votes.clear();
        }

        self.last_heartbeat_millis = current_millis;
        self.timeout_at_millis = current_millis + self.election_timeout_millis();
        self.known_leader_id = Some(req.leader_id);

        // A leader may retry from an index that this follower has already
        // compacted. Normalize an overlapping request to the snapshot anchor
        // before running the ordinary log-matching loop. Appending the old
        // prefix after the retained suffix would shift every logical index
        // and can turn unrelated application bytes into configuration entries.
        let snapshot_index = self.log_store.snapshot_index();
        if req.prev_log_index < snapshot_index {
            let overlap = snapshot_index.saturating_sub(req.prev_log_index) as usize;
            if req.entries.len() < overlap {
                return AppendEntriesResponse {
                    peer_id: self.me.id.clone(),
                    term: self.current_term,
                    success: true,
                    match_index: snapshot_index,
                };
            }
            if req.entries[overlap - 1].term != self.log_store.snapshot_term() {
                return AppendEntriesResponse {
                    peer_id: self.me.id.clone(),
                    term: self.current_term,
                    success: false,
                    match_index: snapshot_index,
                };
            }
            req.entries.drain(0..overlap);
            req.prev_log_index = snapshot_index;
            req.prev_log_term = self.log_store.snapshot_term();
        }

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

        self.last_successful_leader_sync_millis = Some(current_millis);

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

        if self.state != NodeState::Leader
            || !self.accepts_replication_response(peer_id, &resp.peer_id)
        {
            return;
        }

        if resp.term > self.current_term {
            self.step_down(resp.term, current_millis);
            return;
        }
        if resp.term != self.current_term {
            return;
        }

        if resp.success {
            if resp.match_index > self.log_store.last_index() {
                return;
            }
            self.record_replication_progress(peer_id, resp.match_index);
            self.last_follower_contact_millis.insert(peer_id.to_string(), current_millis);
            self.advance_commit_index();
            self.maybe_auto_finalize_joint_configuration();
        } else {
            let confirmed_next =
                self.match_index.get(peer_id).copied().unwrap_or(0).saturating_add(1);
            if let Some(ni) = self.next_index.get_mut(peer_id) {
                *ni = ni.saturating_sub(1).max(confirmed_next);
            }
        }
    }

    fn accepts_replication_response(&self, peer_id: &str, reported_peer_id: &str) -> bool {
        // The transport supplies the source peer. Replication also serves
        // committed joiners before they enter the active membership.
        peer_id == reported_peer_id
            && peer_id != self.me.id
            && (self.cluster_configuration.contains(peer_id)
                || self.pending_joiners.contains_key(peer_id))
    }

    fn record_replication_progress(&mut self, peer_id: &str, index: u64) {
        // A delayed success may describe an older prefix. Keep the strongest
        // acknowledgement in this leadership term, including snapshot replies.
        let confirmed = self.match_index.entry(peer_id.to_string()).or_insert(0);
        *confirmed = (*confirmed).max(index);
        let next = self.next_index.entry(peer_id.to_string()).or_insert(1);
        *next = (*next).max(confirmed.saturating_add(1));
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
            || (!self.cluster_configuration.is_voter(&req.leader_id)
                && !self.accepts_joining_leader(&req.leader_id))
            || (req.term == self.current_term
                && self.state == NodeState::Follower
                && self.known_leader_id.as_deref().is_some_and(|leader| leader != req.leader_id))
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
            self.last_successful_leader_sync_millis = None;
            self.granted_votes.clear();
        }

        self.last_heartbeat_millis = current_millis;
        self.timeout_at_millis = current_millis + self.election_timeout_millis();
        self.known_leader_id = Some(req.leader_id.clone());

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
                leader_id: req.leader_id.clone(),
                leader_term: req.term,
                last_included_index: req.last_included_index,
                last_included_term: req.last_included_term,
                offset: 0,
                data: Vec::new(),
            });
        } else if self
            .pending_snapshot
            .as_ref()
            .is_some_and(|snap| snap.leader_id != req.leader_id || snap.leader_term != req.term)
        {
            // Boundary/offset equality does not identify a stream. Different
            // leaders can encode the same snapshot state differently.
            self.pending_snapshot = None;
        }

        let mut accepted = false;
        let mut next_offset = 0;
        let mut oversized = false;
        if let Some(ref mut snap) = self.pending_snapshot {
            next_offset = snap.offset;
            if req.leader_id == snap.leader_id
                && req.term == snap.leader_term
                && req.last_included_index == snap.last_included_index
                && req.last_included_term == snap.last_included_term
                && req.offset == snap.offset
            {
                if snap.data.len().saturating_add(req.data.len()) > crate::types::MAX_SNAPSHOT_BYTES
                {
                    oversized = true;
                } else {
                    snap.data.extend_from_slice(&req.data);
                    snap.offset += req.data.len() as u64;
                    next_offset = snap.offset;
                    accepted = true;
                }
            }
        }
        if oversized {
            // Drop the partial stream and tell the leader to restart so a
            // hostile or buggy peer cannot grow follower memory without bound.
            self.pending_snapshot = None;
            return InstallSnapshotResponse {
                peer_id: self.me.id.clone(),
                term: self.current_term,
                success: false,
                last_included_index: req.last_included_index,
                next_offset: 0,
                done: false,
            };
        }

        let installed = accepted && req.done;
        if installed {
            if let Some(ref snap) = self.pending_snapshot {
                let data = snap.data.clone();
                let envelope = crate::snapshot_codec::decode_snapshot_payload(&data);
                let application_data = envelope
                    .as_ref()
                    .map_or(data.as_slice(), |envelope| envelope.state_machine_snapshot.as_slice());
                if let Some(ref machine) = self.state_machine
                    && machine.restore(application_data).is_err()
                {
                    self.pending_snapshot = None;
                    return InstallSnapshotResponse {
                        peer_id: self.me.id.clone(),
                        term: self.current_term,
                        success: false,
                        last_included_index: req.last_included_index,
                        next_offset: 0,
                        done: false,
                    };
                }
                self.log_store.install_snapshot(
                    snap.last_included_index,
                    snap.last_included_term,
                    data.clone(),
                );
                self.commit_index = snap.last_included_index;
                self.last_applied = snap.last_included_index;
                if let Some(envelope) = envelope
                    && !envelope.current_members.is_empty()
                {
                    let mut configuration = ClusterConfiguration::stable(envelope.current_members);
                    if !envelope.next_members.is_empty() {
                        configuration = configuration.transition_to(envelope.next_members);
                    }
                    self.snapshot_configuration = configuration.clone();
                    self.cluster_configuration = configuration.clone();
                    self.membership_epoch = if envelope.membership_epoch == 0 {
                        snap.last_included_index
                    } else {
                        envelope.membership_epoch
                    };
                    self.committed_configurations
                        .retain(|index, _| *index > snap.last_included_index);
                    self.committed_configurations.insert(snap.last_included_index, configuration);
                    self.decommissioned = !self.cluster_configuration.contains(&self.me.id);
                    if !self.decommissioned {
                        self.persistent_state.set_join_admission(None);
                        self.joining = false;
                        self.joining_from = None;
                    }
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
            }
            self.pending_snapshot = None;
            self.last_successful_leader_sync_millis = Some(current_millis);
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

    /// Consume one asynchronous InstallSnapshot outcome on the leader side.
    pub fn handle_install_snapshot_response(
        &mut self,
        peer_id: &str,
        response: InstallSnapshotResponse,
        sent_next_offset: u64,
        sent_done: bool,
        current_millis: u64,
    ) {
        self.current_millis = current_millis;
        if !self.accepts_replication_response(peer_id, &response.peer_id) {
            return;
        }
        if response.term > self.current_term {
            self.step_down(response.term, current_millis);
        } else if self.state == NodeState::Leader && response.term == self.current_term {
            if response.success {
                if response.last_included_index > self.log_store.last_index() {
                    return;
                }
                self.snapshot_offsets.insert(peer_id.to_string(), sent_next_offset);
                if sent_done && response.done {
                    self.record_replication_progress(peer_id, response.last_included_index);
                    self.snapshot_offsets.insert(peer_id.to_string(), 0);
                    self.advance_commit_index();
                }
            } else {
                // The follower rejected the chunk and reports the offset it
                // can accept next (zero once its pending buffer is gone).
                // Rewind so a restarted or partially-buffered follower
                // resumes instead of rejecting every later chunk.
                self.snapshot_offsets.insert(peer_id.to_string(), response.next_offset);
            }
        }
    }

    pub fn broadcast_heartbeat(&mut self, current_millis: u64) {
        self.current_millis = current_millis;
        self.transport.set_current_millis(current_millis);

        if self.state != NodeState::Leader {
            return;
        }

        // Replicate to configured members and to peers admitted by a
        // committed JOIN entry but not yet promoted. Replication to a peer
        // whose transport connection is not yet wired is a no-op, so
        // iterating the union here is safe before the joiner is reachable.
        let mut replication_peers: Vec<Peer> =
            self.cluster_configuration.all_members().into_iter().cloned().collect();
        for (peer, _) in self.pending_joiners.values() {
            if !replication_peers.iter().any(|member| member.id == peer.id) {
                replication_peers.push(peer.clone());
            }
        }

        for peer in &replication_peers {
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
        self.maybe_promote_pending_joiners(current_millis);
    }

    /// Promote a joiner into the joint configuration once it has caught up
    /// to its committed JOIN fence. The joint phase then auto-finalizes via
    /// [`maybe_auto_finalize_joint_configuration`] once every proposed remote
    /// member reaches the committed joint fence. Commit needs both voter quorums.
    fn maybe_promote_pending_joiners(&mut self, current_millis: u64) {
        if self.state != NodeState::Leader
            || self.pending_joiners.is_empty()
            || self.cluster_configuration.is_joint_consensus()
            || self.finalize_configuration_pending
        {
            return;
        }
        let ready: Vec<(String, Peer)> = self
            .pending_joiners
            .iter()
            .filter(|(id, (_, fence))| self.match_index.get(*id).copied().unwrap_or(0) >= *fence)
            .map(|(id, (peer, _))| (id.clone(), peer.clone()))
            .collect();
        if ready.is_empty() {
            return;
        }
        let mut members: Vec<Peer> =
            self.cluster_configuration.all_members().into_iter().cloned().collect();
        for (_, peer) in &ready {
            if !members.iter().any(|member| member.id == peer.id) {
                members.push(peer.clone());
            }
        }
        let _ = self.submit_joint_configuration(members, current_millis);
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
                    self.handle_install_snapshot_response(
                        &peer_id,
                        response,
                        sent_next_offset,
                        sent_done,
                        current_millis,
                    );
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
        if self.snapshot_min_entries == 0
            || self.state_machine.is_none()
            || self.commit_index == 0
            // A committed JOIN is the durable admission record until the
            // joiner is promoted into a snapshottable configuration. Do not
            // compact that fence out from underneath pending admission.
            || !self.pending_joiners.is_empty()
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
        let snapshot = crate::snapshot_codec::wrap_snapshot_payload_with_membership_epoch(
            &current_members,
            &next_members,
            self.membership_epoch,
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

    /// Make the membership that completed this node's admission durable
    /// before clearing its durable join fence. Without this ordering, a crash
    /// after applying the joint entry but before ordinary compaction could
    /// resurrect the launch-time singleton configuration.
    fn snapshot_admitted_membership(&mut self, index: u64) {
        let current_members =
            self.cluster_configuration.current_members().into_iter().cloned().collect::<Vec<_>>();
        let next_members = if self.cluster_configuration.is_joint_consensus() {
            self.cluster_configuration.next_members().into_iter().cloned().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let application_snapshot =
            self.state_machine.as_ref().map(|machine| machine.snapshot()).unwrap_or_default();
        let snapshot = crate::snapshot_codec::wrap_snapshot_payload_with_membership_epoch(
            &current_members,
            &next_members,
            index,
            &application_snapshot,
        );
        let term = self.log_store.term_at(index);
        self.log_store.install_snapshot(index, term, snapshot);
        self.snapshot_configuration = self.cluster_configuration.clone();
        self.committed_configurations.retain(|config_index, _| *config_index > index);
    }

    fn apply_configuration_command(&mut self, index: u64, data: &[u8]) -> bool {
        let Some(command) = configuration::decode(data) else {
            return false;
        };
        if let ConfigurationCommand::Join(ref peer) = command {
            // The fence is this entry's index: the joiner must be replicated
            // up to the JOIN before it is promoted into the joint phase.
            self.pending_joiners.insert(peer.id.clone(), (peer.clone(), index));
            return true;
        }
        let entered_joint = matches!(&command, ConfigurationCommand::Joint(_));
        self.cluster_configuration = match command {
            ConfigurationCommand::Joint(members) => {
                // Until this joint entry is applied, newly promoted peers
                // are not in the active configuration and must remain in
                // `pending_joiners` so heartbeat replication still targets
                // them. Once applied, `all_members()` takes over that role.
                self.pending_joiners.retain(|id, _| !members.iter().any(|peer| &peer.id == id));
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
        self.membership_epoch = index;
        self.committed_configurations.insert(index, self.cluster_configuration.clone());
        self.decommissioned = !self.cluster_configuration.contains(&self.me.id);
        if self.cluster_configuration.contains(&self.me.id) {
            if self.joining {
                self.snapshot_admitted_membership(index);
                self.persistent_state.set_join_admission(None);
            }
            self.joining = false;
            self.joining_from = None;
        }
        if !self.cluster_configuration.is_voter(&self.me.id) {
            // Finalization can retain us as a learner. Relinquish leadership
            // immediately while preserving this term's durable vote.
            self.state = NodeState::Follower;
            self.known_leader_id = None;
            self.last_successful_leader_sync_millis = None;
            self.last_follower_contact_millis.clear();
            self.granted_votes.clear();
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
        if entered_joint {
            self.maybe_auto_finalize_joint_configuration();
        }
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
        if fence == 0
            || self.commit_index < fence
            || self.pending_auto_finalize_members.iter().any(|id| {
                id != &self.me.id && self.match_index.get(id).copied().unwrap_or(0) < fence
            })
        {
            return;
        }
        // Match Graft's catch-up policy before proposing FINALIZE. The leader
        // owns the committed entry locally; every proposed remote voter or
        // learner must acknowledge it. Commit still needs both voter quorums.
        if self.submit_finalize_configuration(self.current_millis).is_ok() {
            self.finalize_configuration_pending = true;
        }
    }

    fn has_uncommitted_configuration_change(&self) -> bool {
        (self.commit_index.saturating_add(1)..=self.log_store.last_index()).any(|index| {
            self.log_store.entry_at(index).is_some_and(|entry| {
                matches!(
                    configuration::decode(&entry.data),
                    Some(ConfigurationCommand::Joint(_) | ConfigurationCommand::Finalize)
                )
            })
        })
    }

    pub fn submit_joint_configuration(
        &mut self,
        members: Vec<Peer>,
        current_millis: u64,
    ) -> Result<u64, i64> {
        if self.cluster_configuration.is_joint_consensus()
            || self.finalize_configuration_pending
            || self.has_uncommitted_configuration_change()
        {
            return Err(crate::types::ERR_CONFIG_IN_PROGRESS);
        }
        let command =
            configuration::encode(&ConfigurationCommand::Joint(members)).ok_or(ERR_TOO_LARGE)?;
        let index = self.submit_command(command, current_millis)?;
        self.maybe_auto_finalize_joint_configuration();
        Ok(index)
    }

    pub fn submit_join(&mut self, peer: Peer, current_millis: u64) -> Result<u64, i64> {
        if self.cluster_configuration.contains(&peer.id) {
            return Ok(self.commit_index);
        }
        if let Some((_, fence)) = self.pending_joiners.get(&peer.id) {
            return Ok(*fence);
        }
        // A retried network request can arrive before its original JOIN has
        // committed. Reuse that log index rather than appending duplicates.
        for index in (self.log_store.snapshot_index() + 1)..=self.log_store.last_index() {
            if let Some(entry) = self.log_store.entry_at(index)
                && matches!(
                    configuration::decode(&entry.data),
                    Some(ConfigurationCommand::Join(ref existing)) if existing.id == peer.id
                )
            {
                return Ok(index);
            }
        }
        let command =
            configuration::encode(&ConfigurationCommand::Join(peer)).ok_or(ERR_TOO_LARGE)?;
        self.submit_command(command, current_millis)
    }

    /// Peers admitted by committed JOIN entries that have not yet been
    /// promoted. The service layer wires transport connections for these
    /// peers so the leader can replicate them up to their fences.
    pub fn pending_joiners(&self) -> Vec<Peer> {
        self.pending_joiners.values().map(|(peer, _)| peer.clone()).collect()
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

    /// Receive-time freshness for bounded observations; grants no query authority.
    fn has_recent_quorum_contact(&self) -> bool {
        // Committing in this term establishes the inherited committed prefix.
        // Apply that prefix before bounded observation, including a compacted anchor.
        // Recent contact is a separate gate; a delayed same-term response can
        // refresh it after another quorum replaces this leader.
        if self.state != NodeState::Leader
            || self.commit_index == 0
            || self.log_store.term_at(self.commit_index) != self.current_term
            || self.last_applied != self.commit_index
        {
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

    /// Whether local committed state is backed by recent cluster evidence.
    ///
    /// Leaders use receive-time quorum freshness, which grants no query authority.
    /// Followers require a successful log match from the recognized leader;
    /// rejected heartbeats do not count. Both are receive-time observations.
    pub fn can_serve_bounded_read(&self, max_follower_age_millis: u64) -> bool {
        if self.state == NodeState::Leader {
            return self.has_recent_quorum_contact();
        }
        self.state == NodeState::Follower
            && !self.joining
            && !self.decommissioned
            && self.known_leader_id.is_some()
            && self.commit_index == self.last_applied
            && self.last_successful_leader_sync_millis.is_some_and(|last_sync| {
                self.current_millis.saturating_sub(last_sync) <= max_follower_age_millis
            })
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

    /// Admit one read before appending a fresh no-op in the shared Raft grammar.
    /// Existing contact or an earlier request's acknowledgement cannot commit
    /// this new index. The caller retains the owner while servicing Raft traffic.
    pub fn begin_read(&mut self, query: Vec<u8>, current_millis: u64) -> Result<PendingRead, i64> {
        self.begin_read_with(query, current_millis, || {
            Arc::try_new(()).map_err(|_| crate::types::ERR_RESOURCE_LIMIT)
        })
    }

    fn begin_read_with(
        &mut self,
        query: Vec<u8>,
        current_millis: u64,
        prepare_domain: impl FnOnce() -> Result<Arc<()>, i64>,
    ) -> Result<PendingRead, i64> {
        if self.state != NodeState::Leader
            || self.joining
            || self.decommissioned
            || !self.cluster_configuration.is_voter(&self.me.id)
        {
            return Err(ERR_NOT_LEADER);
        }
        if query.len() > MAX_COMMAND_BYTES || self.log_store.last_index() == u64::MAX {
            return Err(ERR_TOO_LARGE);
        }
        if self.read_domain.is_none() {
            self.read_domain = Some(prepare_domain()?);
        }
        let read = PendingRead {
            domain: self.read_domain.as_ref().unwrap().clone(),
            term: self.current_term,
            index: self.log_store.last_index() + 1,
            membership_epoch: self.membership_epoch,
            query,
        };
        self.current_millis = current_millis;
        self.log_store.append(alloc::vec![LogEntry::noop(self.current_term, self.me.id.clone())]);
        self.advance_commit_index();
        Ok(read)
    }

    /// Observe progress without executing the query or consuming its owner.
    pub fn read_ready(&self, read: &PendingRead) -> Result<bool, i64> {
        if self.state != NodeState::Leader
            || self.current_term != read.term
            || self.joining
            || self.decommissioned
        {
            return Err(ERR_NOT_LEADER);
        }
        if self.membership_epoch != read.membership_epoch
            || !self.read_domain.as_ref().is_some_and(|domain| Arc::ptr_eq(domain, &read.domain))
        {
            return Err(ERR_READ_BARRIER);
        }
        Ok(self.commit_index >= read.index
            && self.last_applied == self.commit_index
            && self.log_store.term_at(self.commit_index) == read.term)
    }

    pub fn handle_client_query(&self, read: PendingRead) -> Result<Vec<u8>, (i64, Option<String>)> {
        match self.read_ready(&read) {
            Ok(true) => {}
            Ok(false) => return Err((ERR_READ_BARRIER, self.known_leader_id.clone())),
            Err(code) => return Err((code, self.known_leader_id.clone())),
        }

        if let Some(ref sm) = self.state_machine
            && let Some(qs) = sm.as_queryable()
        {
            return Ok(qs.query(&read.query));
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
        PendingSnapshot,
        RaftNode,
        RaftNodeConfig,
    };
    use crate::{
        log_store::{
            InMemoryLogStore,
            InMemoryPersistentStateStore,
            LogStore,
            PersistentStateStore,
        },
        membership::ClusterConfiguration,
        state_machine::StateMachine,
        transport::NoopTransport,
        types::{
            AppendEntriesRequest,
            AppendEntriesResponse,
            InstallSnapshotRequest,
            InstallSnapshotResponse,
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

        fn restore(&self, snapshot_data: &[u8]) -> Result<(), crate::state_machine::SnapshotError> {
            *self.restored.lock() = snapshot_data.to_vec();
            Ok(())
        }

        fn reset(&self) {
            self.restored.lock().clear();
        }
    }

    struct ApplyingStateMachine {
        applied: Arc<spin::Mutex<Vec<Vec<u8>>>>,
    }

    impl StateMachine for ApplyingStateMachine {
        fn apply(&self, _term: u64, command: &[u8]) {
            self.applied.lock().push(command.to_vec());
        }

        fn reset(&self) {
            self.applied.lock().clear();
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
        .unwrap()
    }

    fn elected_three_node_leader() -> RaftNode {
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
        node
    }

    fn acknowledge(node: &mut RaftNode, peer: &str, index: u64) {
        node.handle_append_entries_response(
            peer,
            AppendEntriesResponse {
                peer_id: peer.to_string(),
                term: node.current_term,
                success: true,
                match_index: index,
            },
            node.millis(),
        );
    }

    struct CountingQueries(Arc<spin::Mutex<u64>>);

    impl StateMachine for CountingQueries {
        fn apply(&self, _term: u64, _command: &[u8]) {}

        fn reset(&self) {}

        fn as_queryable(&self) -> Option<&dyn crate::state_machine::QueryableStateMachine> {
            Some(self)
        }
    }

    impl crate::state_machine::QueryableStateMachine for CountingQueries {
        fn query(&self, query: &[u8]) -> Vec<u8> {
            *self.0.lock() += 1;
            query.to_vec()
        }
    }

    #[test]
    fn each_read_needs_its_own_committed_fence_and_executes_once() {
        let mut node = elected_three_node_leader();
        let queries = Arc::new(spin::Mutex::new(0));
        node.state_machine = Some(Box::new(CountingQueries(queries.clone())));
        acknowledge(&mut node, "n2", 1);
        let first = node.begin_read(vec![10], 202).unwrap();
        let second = node.begin_read(vec![20], 202).unwrap();
        assert_eq!((first.index, second.index), (2, 3));
        acknowledge(&mut node, "n2", 1);
        assert_eq!(node.read_ready(&first), Ok(false));
        assert_eq!(node.read_ready(&second), Ok(false));
        acknowledge(&mut node, "n2", 2);
        assert_eq!(node.read_ready(&first), Ok(true));
        assert_eq!(node.read_ready(&second), Ok(false));
        assert_eq!(*queries.lock(), 0, "observing readiness does not execute a query");
        assert_eq!(node.handle_client_query(first), Ok(vec![10]));
        assert_eq!(*queries.lock(), 1);
        drop(second);
        acknowledge(&mut node, "n2", 3);
        assert_eq!(node.commit_index, 3, "abandonment does not roll back the no-op");
        assert_eq!(*queries.lock(), 1, "abandoned reads never execute later");
    }

    #[test]
    fn read_fence_rejects_unapplied_commit_and_survives_compaction() {
        let mut node = elected_three_node_leader();
        let read = node.begin_read(Vec::new(), 202).unwrap();
        acknowledge(&mut node, "n2", read.index);
        node.last_applied -= 1;
        assert_eq!(node.read_ready(&read), Ok(false));
        node.last_applied = node.commit_index;
        node.submit_command(vec![1], 203).unwrap();
        let tail = node.log_store.last_index();
        acknowledge(&mut node, "n2", tail);
        node.log_store.install_snapshot(node.commit_index, node.current_term, Vec::new());
        assert_eq!(
            node.log_store.term_at(read.index),
            0,
            "the fence is below the compacted anchor"
        );
        assert_eq!(node.read_ready(&read), Ok(true));
        assert_eq!(node.handle_client_query(read), Ok(Vec::new()));
    }

    #[test]
    fn read_fence_is_bound_to_node_domain_term_and_membership() {
        let mut node = node_with_voters(&["n1"]);
        node.start_election(200);
        let read = node.begin_read(Vec::new(), 201).unwrap();
        let mut other = node_with_voters(&["n1"]);
        other.start_election(200);
        let _other_read = other.begin_read(Vec::new(), 201).unwrap();
        assert_eq!(other.read_ready(&read), Err(crate::types::ERR_READ_BARRIER));
        node.membership_epoch += 1;
        assert_eq!(node.read_ready(&read), Err(crate::types::ERR_READ_BARRIER));
        node.membership_epoch -= 1;
        node.step_down(node.current_term + 1, 202);
        assert_eq!(node.read_ready(&read), Err(crate::types::ERR_NOT_LEADER));
        node.start_election(500);
        assert_eq!(node.read_ready(&read), Err(crate::types::ERR_NOT_LEADER));
        let read = node.begin_read(Vec::new(), 501).unwrap();
        node.begin_joining("n2".to_string(), 502);
        assert_eq!(node.read_ready(&read), Err(crate::types::ERR_NOT_LEADER));
        assert!(node.read_domain.is_none());
    }

    #[test]
    fn read_preparation_failure_and_premature_finish_do_not_query_or_append() {
        let mut node = elected_three_node_leader();
        let tail = node.log_store.last_index();
        assert!(matches!(
            node.begin_read_with(Vec::new(), 202, || Err(crate::types::ERR_RESOURCE_LIMIT)),
            Err(crate::types::ERR_RESOURCE_LIMIT)
        ));
        assert_eq!(node.log_store.last_index(), tail);
        assert!(node.read_domain.is_none());
        let queries = Arc::new(spin::Mutex::new(0));
        node.state_machine = Some(Box::new(CountingQueries(queries.clone())));
        let read = node.begin_read(vec![1], 202).unwrap();
        assert_eq!(
            node.handle_client_query(read),
            Err((crate::types::ERR_READ_BARRIER, Some("n1".to_string())))
        );
        assert_eq!(*queries.lock(), 0);
        node.step_down(node.current_term + 1, 203);
        let tail = node.log_store.last_index();
        assert!(matches!(node.begin_read(Vec::new(), 204), Err(crate::types::ERR_NOT_LEADER)));
        assert_eq!(node.log_store.last_index(), tail);
    }

    #[test]
    fn leader_read_waits_for_current_term_commit_despite_recent_quorum_contact() {
        let mut node = node_with_voters(&["n1", "n2", "n3"]);
        // Inherit an older leader's prefix without its final commit notice.
        // The election no-op must commit before reads can use that state.
        node.log_store.append(vec![LogEntry::noop(1, "n2".to_string())]);
        node.current_term = 1;
        node.persistent_state.set_current_term(1);
        node.start_election(200);
        node.handle_vote_response(
            "n2",
            VoteResponse {
                peer_id: "n2".to_string(),
                term: 2,
                vote_granted: true,
            },
            201,
        );
        assert_eq!(node.state, NodeState::Leader);
        assert_eq!(node.log_store.last_index(), 2);
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                peer_id: "n2".to_string(),
                term: 2,
                success: true,
                match_index: 1,
            },
            202,
        );
        assert_eq!(node.last_follower_contact_millis.get("n2"), Some(&202));
        assert_eq!(node.commit_index, 0);
        assert!(!node.has_recent_quorum_contact());
        assert!(!node.can_serve_bounded_read(1_000));
        // Even a known, applied older-term commit cannot replace the new
        // term's commitment. The inherited entry is a no-op in this fixture.
        node.commit_index = 1;
        node.last_applied = 1;
        assert!(!node.has_recent_quorum_contact());
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                peer_id: "n2".to_string(),
                term: 2,
                success: true,
                match_index: 2,
            },
            203,
        );
        assert_eq!(node.commit_index, 2);
        assert_eq!(node.last_applied, 2);
        assert!(node.has_recent_quorum_contact());
    }

    #[test]
    fn leader_read_requires_applied_commit_and_survives_compaction() {
        let mut node = elected_three_node_leader();
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                peer_id: "n2".to_string(),
                term: node.current_term,
                success: true,
                match_index: 1,
            },
            202,
        );
        assert!(node.has_recent_quorum_contact());
        // Isolate the application gate: the synchronous apply loop normally
        // closes this interval before returning to the service reactor.
        node.last_applied = 0;
        assert!(!node.has_recent_quorum_contact());
        node.last_applied = node.commit_index;
        node.log_store.install_snapshot(node.commit_index, node.current_term, Vec::new());
        assert!(node.log_store.entry_at(node.commit_index).is_none());
        assert!(node.has_recent_quorum_contact());
        node.set_millis(952);
        assert!(!node.has_recent_quorum_contact(), "committing does not bypass contact expiry");
    }

    #[test]
    fn single_voter_read_requires_its_election_noop() {
        let mut node = node_with_voters(&["n1"]);
        assert!(!node.has_recent_quorum_contact());
        node.start_election(200);
        assert_eq!(node.commit_index, 1);
        assert_eq!(node.last_applied, 1);
        assert!(node.has_recent_quorum_contact());
    }

    #[test]
    fn reactor_pause_expires_old_quorum_contact() {
        let mut node = elected_three_node_leader();
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                peer_id: "n2".to_string(),
                term: node.current_term,
                success: true,
                match_index: 1,
            },
            202,
        );
        assert!(node.has_recent_quorum_contact());
        let mut clock = crate::clock::MonotonicClock::new(1_000, 1_000).unwrap();
        assert_eq!(clock.sample(1_202, 1_000), Ok(node.millis()));
        // The next notification follows a five-second reactor pause.
        node.set_millis(clock.sample(6_202, 1_000).unwrap());
        assert!(!node.has_recent_quorum_contact());
        node.state = NodeState::Follower;
        assert!(node.check_timeout(), "election time also includes the pause");
    }

    #[test]
    fn finalized_learner_relinquishes_leadership_without_erasing_its_vote() {
        let mut node = elected_three_node_leader();
        let term = node.current_term;
        let joint = node
            .submit_joint_configuration(
                vec![
                    Peer::learner("n1".to_string(), 0),
                    Peer::voter("n2".to_string(), 0),
                    Peer::voter("n3".to_string(), 0),
                ],
                202,
            )
            .unwrap();
        let acknowledge = |node: &mut RaftNode, peer: &str, index, millis| {
            node.handle_append_entries_response(
                peer,
                AppendEntriesResponse {
                    peer_id: peer.to_string(),
                    term,
                    success: true,
                    match_index: index,
                },
                millis,
            );
        };
        acknowledge(&mut node, "n2", joint, 203);
        assert_eq!(node.commit_index, joint);
        assert!(node.cluster_configuration.is_joint_consensus());
        assert!(node.cluster_configuration.is_voter("n1"));
        assert_eq!(node.state, NodeState::Leader);
        assert_eq!(node.log_store.last_index(), joint);
        acknowledge(&mut node, "n3", joint, 204);
        let finalize = node.log_store.last_index();
        assert!(finalize > joint);
        acknowledge(&mut node, "n2", finalize, 205);
        assert_eq!(node.commit_index, joint, "the new voter majority is still missing");
        acknowledge(&mut node, "n3", finalize, 206);

        assert_eq!(node.commit_index, finalize);
        assert!(!node.cluster_configuration.is_joint_consensus());
        assert!(node.cluster_configuration.contains("n1"));
        assert!(!node.cluster_configuration.is_voter("n1"));
        assert!(!node.decommissioned, "a retained learner remains a cluster member");
        assert_eq!(node.state, NodeState::Follower);
        assert!(node.known_leader_id.is_none());
        assert!(node.last_follower_contact_millis.is_empty());
        assert!(!node.has_recent_quorum_contact());
        assert!(!node.can_serve_bounded_read(750));
        assert_eq!(node.submit_command(vec![0xff], 206), Err(crate::types::ERR_NOT_LEADER));
        assert!(matches!(node.begin_read(Vec::new(), 206), Err(crate::types::ERR_NOT_LEADER)));
        assert_eq!(node.current_term, term);
        assert_eq!(node.voted_for.as_deref(), Some("n1"));
        assert_eq!(node.persistent_state.voted_for().as_deref(), Some("n1"));
        node.start_election(2_000);
        assert_eq!(node.state, NodeState::Follower);
        assert_eq!(node.current_term, term);
        assert!(!node.check_timeout());
    }

    #[test]
    fn finalization_waits_for_a_slow_learner_then_commits() {
        let mut node = elected_three_node_leader();
        let joint = node
            .submit_joint_configuration(
                vec![
                    Peer::voter("n1".to_string(), 0),
                    Peer::voter("n2".to_string(), 0),
                    Peer::learner("n3".to_string(), 0),
                ],
                202,
            )
            .unwrap();
        for millis in [203, 204] {
            node.handle_append_entries_response(
                "n2",
                AppendEntriesResponse {
                    peer_id: "n2".to_string(),
                    term: node.current_term,
                    success: true,
                    match_index: joint,
                },
                millis,
            );
        }
        assert_eq!(node.commit_index, joint);
        assert_eq!(node.log_store.last_index(), joint, "a slow learner delays the proposal");
        assert!(node.cluster_configuration.is_joint_consensus());
        assert!(!node.finalize_configuration_pending);
        node.handle_append_entries_response(
            "n3",
            AppendEntriesResponse {
                peer_id: "n3".to_string(),
                term: node.current_term,
                success: true,
                match_index: joint,
            },
            205,
        );
        assert_eq!(node.log_store.last_index(), joint + 1);
        assert!(node.finalize_configuration_pending);
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                peer_id: "n2".to_string(),
                term: node.current_term,
                success: true,
                match_index: joint + 1,
            },
            206,
        );
        assert_eq!(node.commit_index, joint + 1);
        assert!(!node.cluster_configuration.is_joint_consensus());
        assert_eq!(node.match_index.get("n3"), Some(&joint));
        assert!(node.cluster_configuration.contains("n3"));
        assert!(!node.cluster_configuration.is_voter("n3"));
        assert_eq!(node.state, NodeState::Leader);
    }

    #[test]
    fn next_only_voter_needs_both_joint_election_majorities() {
        let configuration = ClusterConfiguration::stable(vec![
            Peer::voter("n1".to_string(), 0),
            Peer::voter("n2".to_string(), 0),
            Peer::learner("n3".to_string(), 0),
        ])
        .transition_to(vec![Peer::voter("n2".to_string(), 0), Peer::voter("n3".to_string(), 0)]);
        let mut node = RaftNode::new(RaftNodeConfig {
            me: Peer::learner("n3".to_string(), 0),
            timeout_millis: 150,
            log_store: Box::new(InMemoryLogStore::new()),
            persistent_state: Box::new(InMemoryPersistentStateStore::new()),
            state_machine: None,
            cluster_configuration: configuration,
            transport: Arc::new(NoopTransport),
            current_millis: 0,
            snapshot_min_entries: 64,
            snapshot_chunk_bytes: 3000,
        })
        .unwrap();
        node.start_election(200);
        assert_eq!(node.state, NodeState::Candidate);
        for (peer, expected_state) in [("n2", NodeState::Candidate), ("n1", NodeState::Leader)] {
            node.handle_vote_response(
                peer,
                VoteResponse {
                    peer_id: peer.to_string(),
                    term: node.current_term,
                    vote_granted: true,
                },
                201,
            );
            assert_eq!(node.state, expected_state);
        }
    }

    #[test]
    fn old_term_append_response_cannot_commit_a_replaced_prefix() {
        let mut node = elected_three_node_leader();
        node.submit_command(vec![0xff, 1], 202).unwrap();
        let old_index = node.submit_command(vec![0xff, 2], 203).unwrap();
        let delayed = AppendEntriesResponse {
            peer_id: "n2".to_string(),
            term: node.current_term,
            success: true,
            match_index: old_index,
        };
        // n2 acknowledged this old prefix, but its response remains in flight.
        // A different leader replaces it with a shorter, higher-term prefix.
        let replacement_term = node.current_term + 1;
        let response = node.handle_append_entries(
            AppendEntriesRequest {
                term: replacement_term,
                leader_id: "n3".to_string(),
                prev_log_index: 0,
                prev_log_term: 0,
                entries: vec![
                    LogEntry::new(replacement_term, "n3".to_string(), vec![0xff, 3]),
                    LogEntry::new(replacement_term, "n3".to_string(), vec![0xff, 4]),
                ],
                leader_commit: 0,
            },
            300,
        );
        assert!(response.success);
        node.start_election(500);
        node.handle_vote_response(
            "n2",
            VoteResponse {
                peer_id: "n2".to_string(),
                term: node.current_term,
                vote_granted: true,
            },
            501,
        );
        assert_eq!(node.state, NodeState::Leader);
        assert_eq!(node.log_store.last_index(), old_index);
        assert_eq!(node.commit_index, 0);
        let next_index = node.next_index.get("n2").copied();

        node.handle_append_entries_response("n2", delayed.clone(), 1_500);
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                success: false,
                ..delayed.clone()
            },
            1_500,
        );

        assert_eq!(node.commit_index, 0, "an old response cannot acknowledge the new prefix");
        assert_eq!(node.last_applied, 0);
        assert_eq!(node.match_index.get("n2"), Some(&0));
        assert_eq!(node.next_index.get("n2").copied(), next_index);
        assert!(!node.has_recent_quorum_contact());
        // An actual response from this term can acknowledge the replacement.
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                term: node.current_term,
                ..delayed
            },
            1_501,
        );
        assert_eq!(node.commit_index, old_index);
    }

    #[test]
    fn reordered_replication_responses_preserve_confirmed_progress() {
        let mut node = elected_three_node_leader();
        node.submit_command(vec![0xff, 1], 202).unwrap();
        let index = node.submit_command(vec![0xff, 2], 203).unwrap();
        let acknowledgement = AppendEntriesResponse {
            peer_id: "n2".to_string(),
            term: node.current_term,
            success: true,
            match_index: index,
        };
        node.handle_append_entries_response("n2", acknowledgement.clone(), 204);
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                match_index: 1,
                ..acknowledgement.clone()
            },
            205,
        );
        assert_eq!(node.match_index.get("n2"), Some(&index));
        assert_eq!(node.next_index.get("n2"), Some(&(index + 1)));
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                success: false,
                ..acknowledgement
            },
            206,
        );
        assert_eq!(node.next_index.get("n2"), Some(&(index + 1)));
        node.handle_install_snapshot_response(
            "n2",
            InstallSnapshotResponse {
                peer_id: "n2".to_string(),
                term: node.current_term,
                success: true,
                last_included_index: 1,
                next_offset: 0,
                done: true,
            },
            0,
            true,
            207,
        );
        assert_eq!(node.match_index.get("n2"), Some(&index));
        assert_eq!(node.next_index.get("n2"), Some(&(index + 1)));
    }

    #[test]
    fn replication_responses_require_an_admitted_matching_peer_and_local_index() {
        let mut node = elected_three_node_leader();
        let term = node.current_term;
        let acknowledgement = AppendEntriesResponse {
            peer_id: "n2".to_string(),
            term,
            success: true,
            match_index: node.log_store.last_index(),
        };
        node.handle_append_entries_response("n3", acknowledgement.clone(), 202);
        node.handle_append_entries_response(
            "unknown",
            AppendEntriesResponse {
                peer_id: "unknown".to_string(),
                term: term + 1,
                ..acknowledgement.clone()
            },
            203,
        );
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                match_index: u64::MAX,
                ..acknowledgement.clone()
            },
            204,
        );
        assert_eq!(node.state, NodeState::Leader);
        assert_eq!(node.current_term, term);
        assert_eq!(node.commit_index, 0);
        assert!(node.last_follower_contact_millis.is_empty());
        assert!(!node.match_index.contains_key("unknown"));
        assert_eq!(node.match_index.get("n3"), Some(&0));
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                term: term + 1,
                ..acknowledgement
            },
            205,
        );
        assert_eq!(node.state, NodeState::Follower);
        assert_eq!(node.current_term, term + 1);
    }

    #[test]
    fn snapshot_responses_require_an_admitted_peer_and_completed_result() {
        let mut node = elected_three_node_leader();
        let term = node.current_term;
        let response = InstallSnapshotResponse {
            peer_id: "n2".to_string(),
            term,
            success: true,
            last_included_index: node.log_store.last_index(),
            next_offset: 64,
            done: true,
        };
        let offsets_before = node.snapshot_offsets.clone();
        for (source, reply) in [
            ("n3", response.clone()),
            (
                "unknown",
                InstallSnapshotResponse {
                    peer_id: "unknown".to_string(),
                    term: term + 1,
                    ..response.clone()
                },
            ),
            (
                "n1",
                InstallSnapshotResponse {
                    peer_id: "n1".to_string(),
                    ..response.clone()
                },
            ),
            (
                "n2",
                InstallSnapshotResponse {
                    last_included_index: u64::MAX,
                    ..response.clone()
                },
            ),
            (
                "n2",
                InstallSnapshotResponse {
                    term: term - 1,
                    ..response.clone()
                },
            ),
        ] {
            node.handle_install_snapshot_response(source, reply, 64, true, 202);
        }
        assert_eq!(node.current_term, term);
        assert_eq!(node.state, NodeState::Leader);
        assert_eq!(node.commit_index, 0);
        assert_eq!(node.snapshot_offsets, offsets_before);
        assert_eq!(node.match_index.get("n2"), Some(&0));
        node.handle_install_snapshot_response(
            "n2",
            InstallSnapshotResponse {
                done: false,
                ..response.clone()
            },
            64,
            true,
            203,
        );
        assert_eq!(node.commit_index, 0, "an accepted chunk is not a completed snapshot");
        node.handle_install_snapshot_response("n2", response, 64, true, 204);
        assert_eq!(node.commit_index, 1);
        assert_eq!(node.match_index.get("n2"), Some(&1));
    }

    #[test]
    fn join_admission_promotes_then_finalizes() {
        // Single-voter leader: a joiner admitted by a committed JOIN entry is
        // replicated up to its fence, promoted into the joint configuration,
        // and auto-finalized on the expanded membership.
        let mut node = node_with_voters(&["n1"]);
        node.start_election(200);
        assert_eq!(node.state, NodeState::Leader);

        let joiner = Peer::voter("n2".to_string(), 0);
        let fence = node.submit_join(joiner.clone(), 201).expect("leader accepts the join");
        let last_index = node.log_store.last_index();
        assert_eq!(node.submit_join(joiner.clone(), 201), Ok(fence));
        assert_eq!(node.log_store.last_index(), last_index, "retry must be idempotent");

        // On a single-voter cluster the JOIN commits locally; the joiner is
        // pending admission and not yet a member.
        assert!(node.is_committed(fence));
        assert_eq!(node.pending_joiners().len(), 1);
        assert!(!node.cluster_configuration.contains("n2"));

        // The joiner acks replication up to the fence; the next heartbeat
        // promotes it into the joint configuration.
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                peer_id: "n2".to_string(),
                term: node.current_term,
                success: true,
                match_index: fence,
            },
            202,
        );
        node.broadcast_heartbeat(203);
        assert!(node.pending_joiners().is_empty());
        assert!(node.cluster_configuration.is_joint_consensus());
        assert_eq!(node.cluster_configuration.all_members().len(), 2);

        // The joiner acks the joint entry, committing it and triggering the
        // auto-finalize; the finalize entry then commits once the joiner acks
        // it too, leaving a stable two-member configuration.
        let joint_index = node.log_store.last_index();
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                peer_id: "n2".to_string(),
                term: node.current_term,
                success: true,
                match_index: joint_index,
            },
            204,
        );
        let finalize_index = node.log_store.last_index();
        node.handle_append_entries_response(
            "n2",
            AppendEntriesResponse {
                peer_id: "n2".to_string(),
                term: node.current_term,
                success: true,
                match_index: finalize_index,
            },
            205,
        );
        assert!(!node.cluster_configuration.is_joint_consensus());
        assert_eq!(node.cluster_configuration.all_members().len(), 2);
        assert!(node.cluster_configuration.contains("n2"));
    }

    #[test]
    fn joining_node_accepts_only_its_selected_anchor() {
        let mut anchor = node_with_voters(&["n1"]);
        anchor.start_election(200);
        let joiner_peer = Peer::voter("n2".to_string(), 0);
        anchor.submit_join(joiner_peer, 201).expect("join fence");

        let mut joiner = node_with_voters(&["n2"]);
        joiner.start_election(200);
        assert!(joiner.log_store.last_index() > 0);
        joiner.begin_joining("n1".to_string(), 201);
        assert_eq!(joiner.log_store.last_index(), 0);
        assert_eq!(joiner.commit_index, 0);
        assert_eq!(joiner.last_applied, 0);
        let entries = anchor.log_store.entries_from(1);
        let accepted = joiner.handle_append_entries(
            AppendEntriesRequest {
                term: anchor.current_term,
                leader_id: "n1".to_string(),
                prev_log_index: 0,
                prev_log_term: 0,
                entries: entries.clone(),
                leader_commit: anchor.commit_index,
            },
            202,
        );
        assert!(accepted.success);

        let rejected = joiner.handle_append_entries(
            AppendEntriesRequest {
                term: anchor.current_term + 1,
                leader_id: "attacker".to_string(),
                prev_log_index: 0,
                prev_log_term: 0,
                entries,
                leader_commit: anchor.commit_index,
            },
            203,
        );
        assert!(!rejected.success);
    }

    #[test]
    fn joining_discards_standalone_application_and_replication_state() {
        let restored = Arc::new(spin::Mutex::new(b"standalone".to_vec()));
        let peer = Peer::voter("n2".to_string(), 0);
        let mut node = RaftNode::new(RaftNodeConfig {
            me: peer.clone(),
            timeout_millis: 150,
            log_store: Box::new(InMemoryLogStore::new()),
            persistent_state: Box::new(InMemoryPersistentStateStore::new()),
            state_machine: Some(Box::new(RecordingStateMachine {
                restored: Arc::clone(&restored),
            })),
            cluster_configuration: ClusterConfiguration::stable(vec![peer]),
            transport: Arc::new(NoopTransport),
            current_millis: 0,
            snapshot_min_entries: 64,
            snapshot_chunk_bytes: 3000,
        })
        .unwrap();
        node.start_election(200);
        node.next_index.insert("stale".to_string(), 9);
        node.match_index.insert("stale".to_string(), 8);
        node.snapshot_offsets.insert("stale".to_string(), 7);
        node.pending_snapshot = Some(PendingSnapshot {
            leader_id: "stale".to_string(),
            leader_term: 1,
            last_included_index: 6,
            last_included_term: 1,
            offset: 2,
            data: b"stale".to_vec(),
        });

        node.begin_joining("n1".to_string(), 201);

        assert!(restored.lock().is_empty());
        assert_eq!(node.log_store.last_index(), 0);
        assert!(node.next_index.is_empty());
        assert!(node.match_index.is_empty());
        assert!(node.snapshot_offsets.is_empty());
        assert!(node.pending_snapshot.is_none());
    }

    #[test]
    fn restart_keeps_joiner_fenced_until_new_membership_is_durable() {
        let log_store = InMemoryLogStore::new();
        let persistent_state = InMemoryPersistentStateStore::new();
        let me = Peer::voter("n2".to_string(), 2);

        // A pre-admission singleton snapshot is not proof that the later
        // admission completed, even though it already contains this node.
        let old_snapshot =
            crate::snapshot_codec::wrap_snapshot_payload(core::slice::from_ref(&me), &[], &[]);
        log_store.install_snapshot(2, 1, old_snapshot);
        let make_node = || {
            RaftNode::new(RaftNodeConfig {
                me: me.clone(),
                timeout_millis: 150,
                log_store: Box::new(log_store.clone()),
                persistent_state: Box::new(persistent_state.clone()),
                state_machine: None,
                cluster_configuration: ClusterConfiguration::stable(vec![me.clone()]),
                transport: Arc::new(NoopTransport),
                current_millis: 0,
                snapshot_min_entries: 64,
                snapshot_chunk_bytes: 3000,
            })
            .unwrap()
        };

        let mut node = make_node();
        node.begin_joining("n1".to_string(), 200);
        assert!(persistent_state.join_admission().is_some());
        drop(node);

        let mut restarted = make_node();
        assert!(restarted.joining);
        assert_eq!(restarted.joining_from.as_deref(), Some("n1"));
        restarted.start_election(1_000);
        assert_eq!(restarted.state, NodeState::Follower);
        assert_eq!(restarted.current_term, 0);

        let rejected = restarted.handle_append_entries(
            AppendEntriesRequest {
                term: 2,
                leader_id: "attacker".to_string(),
                prev_log_index: 2,
                prev_log_term: 1,
                entries: Vec::new(),
                leader_commit: 2,
            },
            1_001,
        );
        assert!(!rejected.success);

        // Applying the anchor's joint configuration first snapshots that
        // membership, then clears the durable admission fence. Admission
        // discarded the old standalone index space, so authoritative
        // replication begins at index one.
        let joint =
            crate::configuration::encode(&crate::configuration::ConfigurationCommand::Joint(vec![
                Peer::voter("n1".to_string(), 1),
                me.clone(),
            ]))
            .expect("encode joint configuration");
        let accepted = restarted.handle_append_entries(
            AppendEntriesRequest {
                term: 2,
                leader_id: "n1".to_string(),
                prev_log_index: 0,
                prev_log_term: 0,
                entries: vec![LogEntry::new(2, "n1".to_string(), joint)],
                leader_commit: 1,
            },
            1_002,
        );
        assert!(accepted.success);
        assert!(!restarted.joining);
        assert_eq!(log_store.snapshot_index(), 1);
        assert_eq!(persistent_state.join_admission(), None);
        drop(restarted);

        // This is the other crash edge: the membership snapshot reached
        // stable storage, but clearing the admission record did not. Recovery
        // recognizes only the strictly newer snapshot as completion proof.
        persistent_state.set_join_admission(Some(crate::log_store::JoinAdmission {
            anchor_id: "n1".to_string(),
            snapshot_index: 0,
        }));
        let recovered_member = make_node();
        assert!(!recovered_member.joining);
        assert_eq!(persistent_state.join_admission(), None);
        assert!(recovered_member.cluster_configuration.is_joint_consensus());
        assert!(recovered_member.cluster_configuration.contains("n1"));
        assert!(recovered_member.cluster_configuration.contains("n2"));
        assert_eq!(recovered_member.membership_epoch(), 1);
    }

    #[test]
    fn caught_up_joiners_share_one_joint_transition() {
        let mut node = node_with_voters(&["n1"]);
        node.start_election(200);
        let n2 = node.submit_join(Peer::voter("n2".to_string(), 0), 201).unwrap();
        let n3 = node.submit_join(Peer::voter("n3".to_string(), 0), 202).unwrap();
        node.match_index.insert("n2".to_string(), n2);
        node.match_index.insert("n3".to_string(), n3);

        node.broadcast_heartbeat(203);

        assert!(node.pending_joiners().is_empty());
        assert!(node.cluster_configuration.is_joint_consensus());
        assert_eq!(node.cluster_configuration.next_members().len(), 3);
    }

    #[test]
    fn automatic_finalization_requires_a_committed_nonzero_fence() {
        let mut node = node_with_voters(&["n1", "n2"]);
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

        node.cluster_configuration = node.cluster_configuration.transition_to(vec![
            Peer::voter("n1".to_string(), 0),
            Peer::voter("n2".to_string(), 0),
            Peer::voter("n3".to_string(), 0),
        ]);
        node.pending_auto_finalize_members =
            vec!["n1".to_string(), "n2".to_string(), "n3".to_string()];
        let last_index = node.log_store.last_index();
        node.pending_auto_finalize_fence_index = 0;

        node.maybe_auto_finalize_joint_configuration();

        assert!(!node.finalize_configuration_pending);
        assert_eq!(node.log_store.last_index(), last_index);
        node.pending_auto_finalize_fence_index = 7;

        node.maybe_auto_finalize_joint_configuration();

        assert!(!node.finalize_configuration_pending);
        assert_eq!(node.log_store.last_index(), last_index);
    }

    #[test]
    fn uncommitted_joint_change_blocks_an_overlapping_transition() {
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

        let members = node.cluster_configuration.all_members().into_iter().cloned().collect();
        node.submit_joint_configuration(members, 202).expect("first transition is appended");
        assert!(!node.cluster_configuration.is_joint_consensus());
        assert_eq!(
            node.submit_joint_configuration(vec![Peer::voter("n1".to_string(), 0)], 203),
            Err(crate::types::ERR_CONFIG_IN_PROGRESS)
        );
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
        })
        .unwrap();

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
        })
        .unwrap();

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

    struct RejectingStateMachine;

    impl StateMachine for RejectingStateMachine {
        fn apply(&self, _term: u64, _command: &[u8]) {}

        fn restore(&self, _data: &[u8]) -> Result<(), crate::state_machine::SnapshotError> {
            Err(crate::state_machine::SnapshotError::Unsupported)
        }

        fn reset(&self) {}
    }

    #[test]
    fn restart_rejects_application_snapshot_before_clearing_join_admission() {
        let peers = vec![Peer::voter("n1".to_string(), 0), Peer::voter("n2".to_string(), 0)];
        let store = InMemoryLogStore::new();
        let snapshot = crate::snapshot_codec::wrap_snapshot_payload_with_membership_epoch(
            &peers,
            &[],
            7,
            &[9],
        );
        store.install_snapshot(7, 3, snapshot.clone());
        let persistent = InMemoryPersistentStateStore::new();
        let admission = crate::log_store::JoinAdmission {
            anchor_id: "n2".to_string(),
            snapshot_index: 0,
        };
        persistent.set_current_term(5);
        persistent.set_voted_for(Some("n1".to_string()));
        persistent.set_join_admission(Some(admission.clone()));
        let result = RaftNode::new(RaftNodeConfig {
            me: peers[0].clone(),
            timeout_millis: 150,
            log_store: Box::new(store.clone()),
            persistent_state: Box::new(persistent.clone()),
            state_machine: Some(Box::new(RejectingStateMachine)),
            cluster_configuration: ClusterConfiguration::stable(peers),
            transport: Arc::new(NoopTransport),
            current_millis: 0,
            snapshot_min_entries: 64,
            snapshot_chunk_bytes: 3000,
        });
        assert!(matches!(result, Err(crate::state_machine::SnapshotError::Unsupported)));
        assert_eq!(store.snapshot_data(), snapshot);
        assert_eq!(store.snapshot_index(), 7);
        assert_eq!(persistent.join_admission(), Some(admission));
        assert_eq!(persistent.current_term(), 5);
        assert_eq!(persistent.voted_for().as_deref(), Some("n1"));
    }

    #[test]
    fn rejected_streamed_snapshot_does_not_publish_log_membership_or_join_completion() {
        let mut node = node_with_voters(&["n1", "n2"]);
        node.begin_joining("n2".to_string(), 0);
        node.state_machine = Some(Box::new(RejectingStateMachine));
        let peers = vec![Peer::voter("n1".to_string(), 0), Peer::voter("n3".to_string(), 0)];
        let data = crate::snapshot_codec::wrap_snapshot_payload_with_membership_epoch(
            &peers,
            &[],
            7,
            &[9],
        );
        let split = data.len() / 2;
        let request = InstallSnapshotRequest {
            term: 3,
            leader_id: "n2".to_string(),
            last_included_index: 7,
            last_included_term: 3,
            offset: 0,
            data: data[..split].to_vec(),
            done: false,
        };
        let first = node.handle_install_snapshot(request.clone(), 1);
        assert!(first.success && !first.done);
        assert_eq!(first.next_offset, split as u64);
        let final_response = node.handle_install_snapshot(
            InstallSnapshotRequest {
                offset: split as u64,
                data: data[split..].to_vec(),
                done: true,
                ..request.clone()
            },
            2,
        );
        assert!(!final_response.success && !final_response.done);
        assert_eq!(final_response.next_offset, 0);
        assert!(node.pending_snapshot.is_none());
        assert_eq!(node.log_store.snapshot_index(), 0);
        assert_eq!(node.commit_index, 0);
        assert_eq!(node.last_applied, 0);
        assert_eq!(node.membership_epoch(), 0);
        assert!(node.cluster_configuration.contains("n2"));
        assert!(!node.cluster_configuration.contains("n3"));
        assert!(node.joining);
        assert!(node.persistent_state.join_admission().is_some());
        // A supported replacement can still be installed after explicit rejection.
        let restored = Arc::new(spin::Mutex::new(Vec::new()));
        node.state_machine = Some(Box::new(RecordingStateMachine {
            restored: restored.clone(),
        }));
        let response = node.handle_install_snapshot(
            InstallSnapshotRequest {
                data,
                done: true,
                ..request
            },
            3,
        );
        assert!(response.success && response.done);
        assert_eq!(&*restored.lock(), &[9]);
        assert_eq!(node.log_store.snapshot_index(), 7);
        assert_eq!(node.commit_index, 7);
        assert_eq!(node.last_applied, 7);
        assert_eq!(node.membership_epoch(), 7);
        assert!(!node.joining);
        assert!(node.persistent_state.join_admission().is_none());
    }

    fn assert_snapshot_continuation_rejected(leader: &str) {
        let mut node = node_with_voters(&["n1", "n2", "n3"]);
        let restored = Arc::new(spin::Mutex::new(Vec::new()));
        node.state_machine = Some(Box::new(RecordingStateMachine {
            restored: restored.clone(),
        }));
        let first = InstallSnapshotRequest {
            term: 1,
            leader_id: "n2".to_string(),
            last_included_index: 7,
            last_included_term: 1,
            offset: 0,
            data: b"old-prefix".to_vec(),
            done: false,
        };
        assert!(node.handle_install_snapshot(first.clone(), 1).success);
        let reply = node.handle_install_snapshot(
            InstallSnapshotRequest {
                term: 2,
                leader_id: leader.to_string(),
                offset: first.data.len() as u64,
                data: b"new-suffix".to_vec(),
                done: true,
                ..first.clone()
            },
            2,
        );
        assert!(
            !reply.success && !reply.done,
            "cross-episode continuation from {leader}: restored={:?}, snapshot_index={}, \
             commit={}, applied={}",
            *restored.lock(),
            node.log_store.snapshot_index(),
            node.commit_index,
            node.last_applied,
        );
        assert_eq!(reply.next_offset, 0);
        assert!(node.pending_snapshot.is_none());
        assert!(restored.lock().is_empty());
        assert_eq!(node.log_store.snapshot_index(), 0);
        assert_eq!((node.commit_index, node.last_applied, node.membership_epoch()), (0, 0, 0));
        assert_eq!(node.current_term, 2, "term observation still takes effect");
        let replacement = b"complete replacement".to_vec();
        let reply = node.handle_install_snapshot(
            InstallSnapshotRequest {
                term: 2,
                leader_id: leader.to_string(),
                data: replacement.clone(),
                done: true,
                ..first
            },
            3,
        );
        assert!(reply.success && reply.done);
        assert_eq!(*restored.lock(), replacement);
        assert_eq!(node.commit_index, 7);
    }

    #[test]
    fn snapshot_continuation_cannot_cross_leaders() {
        assert_snapshot_continuation_rejected("n3");
    }

    #[test]
    fn snapshot_continuation_cannot_cross_election_terms() {
        assert_snapshot_continuation_rejected("n2");
    }

    #[test]
    fn snapshot_rejections_preserve_the_current_stream_and_zero_restarts_it() {
        let mut node = node_with_voters(&["n1", "n2", "n3"]);
        let restored = Arc::new(spin::Mutex::new(Vec::new()));
        node.state_machine = Some(Box::new(RecordingStateMachine {
            restored: restored.clone(),
        }));
        let first = InstallSnapshotRequest {
            term: 1,
            leader_id: "n2".to_string(),
            last_included_index: 7,
            last_included_term: 1,
            offset: 0,
            data: b"prefix".to_vec(),
            done: false,
        };
        assert!(node.handle_install_snapshot(first.clone(), 1).success);
        let offset = first.data.len() as u64;
        let continuation = InstallSnapshotRequest {
            offset,
            data: b"suffix".to_vec(),
            done: true,
            ..first.clone()
        };
        for rejected in [
            InstallSnapshotRequest {
                term: 0,
                ..continuation.clone()
            },
            InstallSnapshotRequest {
                leader_id: "n3".to_string(),
                ..continuation.clone()
            },
            InstallSnapshotRequest {
                offset: offset + 1,
                ..continuation.clone()
            },
            InstallSnapshotRequest {
                last_included_index: 8,
                ..continuation.clone()
            },
            InstallSnapshotRequest {
                last_included_term: 2,
                ..continuation.clone()
            },
        ] {
            let reply = node.handle_install_snapshot(rejected, 2);
            assert!(!reply.success && !reply.done);
            assert_eq!(node.pending_snapshot.as_ref().unwrap().data, first.data);
            assert!(restored.lock().is_empty());
            assert_eq!(node.commit_index, 0);
        }
        let reply = node.handle_install_snapshot(continuation, 3);
        assert!(reply.success && reply.done);
        assert_eq!(&*restored.lock(), b"prefixsuffix");
        assert!(node.pending_snapshot.is_none());

        let replacement = InstallSnapshotRequest {
            last_included_index: 9,
            ..first
        };
        assert!(node.handle_install_snapshot(replacement.clone(), 4).success);
        let reply = node.handle_install_snapshot(
            InstallSnapshotRequest {
                data: b"complete".to_vec(),
                done: true,
                ..replacement
            },
            5,
        );
        assert!(reply.success && reply.done);
        assert_eq!(&*restored.lock(), b"complete");
        assert_eq!(node.commit_index, 9);
    }

    #[test]
    fn election_and_indirect_term_observation_discard_partial_snapshot() {
        for transition in 0..3 {
            let mut node = node_with_voters(&["n1", "n2", "n3"]);
            assert!(
                node.handle_install_snapshot(
                    InstallSnapshotRequest {
                        term: 1,
                        leader_id: "n2".to_string(),
                        last_included_index: 7,
                        last_included_term: 1,
                        offset: 0,
                        data: b"prefix".to_vec(),
                        done: false,
                    },
                    1,
                )
                .success
            );
            match transition {
                0 => node.start_election(2),
                1 => {
                    node.handle_vote_request(
                        VoteRequest {
                            term: 2,
                            candidate_id: "n3".to_string(),
                            last_log_index: 0,
                            last_log_term: 0,
                        },
                        2,
                    );
                }
                _ => {
                    node.handle_append_entries(
                        AppendEntriesRequest {
                            term: 2,
                            leader_id: "n3".to_string(),
                            prev_log_index: 0,
                            prev_log_term: 0,
                            entries: Vec::new(),
                            leader_commit: 0,
                        },
                        2,
                    );
                }
            }
            assert_eq!(node.current_term, 2);
            assert!(node.pending_snapshot.is_none());
            assert_eq!(
                (node.log_store.snapshot_index(), node.commit_index, node.last_applied),
                (0, 0, 0)
            );
        }
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
        })
        .unwrap();

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
    fn rejected_snapshot_chunk_rewinds_the_send_offset() {
        // A follower that lost its pending buffer (or has partial data) reports
        // the offset it can accept next. The leader must rewind instead of
        // resending chunks that will be rejected forever.
        let mut node = node_with_voters(&["n1"]);
        node.start_election(200);
        assert_eq!(node.state, NodeState::Leader);
        let term = node.current_term;
        node.submit_join(Peer::voter("n2".to_string(), 0), 200).unwrap();

        node.snapshot_offsets.insert("n2".to_string(), 512);
        node.handle_install_snapshot_response(
            "n2",
            InstallSnapshotResponse {
                peer_id: "n2".to_string(),
                term,
                success: false,
                last_included_index: 0,
                next_offset: 0,
                done: false,
            },
            640,
            false,
            201,
        );
        assert_eq!(
            node.snapshot_offsets.get("n2"),
            Some(&0),
            "a restarted follower rewinds the leader to offset zero"
        );

        node.snapshot_offsets.insert("n2".to_string(), 512);
        node.handle_install_snapshot_response(
            "n2",
            InstallSnapshotResponse {
                peer_id: "n2".to_string(),
                term,
                success: false,
                last_included_index: 0,
                next_offset: 128,
                done: false,
            },
            640,
            false,
            201,
        );
        assert_eq!(
            node.snapshot_offsets.get("n2"),
            Some(&128),
            "a partially buffered follower resumes at its accepted offset"
        );
    }

    #[test]
    fn append_retry_crossing_snapshot_does_not_duplicate_the_compacted_prefix() {
        let store = InMemoryLogStore::new();
        store.append(
            (1..=4).map(|index| LogEntry::new(1, "n2".to_string(), vec![index as u8])).collect(),
        );
        store.install_snapshot(3, 1, Vec::new());
        let peers = vec![Peer::voter("n1".to_string(), 0), Peer::voter("n2".to_string(), 0)];
        let mut node = RaftNode::new(RaftNodeConfig {
            me: peers[0].clone(),
            timeout_millis: 150,
            log_store: Box::new(store),
            persistent_state: Box::new(InMemoryPersistentStateStore::new()),
            state_machine: None,
            cluster_configuration: ClusterConfiguration::stable(peers),
            transport: Arc::new(NoopTransport),
            current_millis: 0,
            snapshot_min_entries: 64,
            snapshot_chunk_bytes: 3000,
        })
        .unwrap();
        let entries =
            (1..=5).map(|index| LogEntry::new(1, "n2".to_string(), vec![index as u8])).collect();

        let response = node.handle_append_entries(
            AppendEntriesRequest {
                term: 1,
                leader_id: "n2".to_string(),
                prev_log_index: 0,
                prev_log_term: 0,
                entries,
                leader_commit: 5,
            },
            200,
        );

        assert!(response.success);
        assert_eq!(response.match_index, 5);
        assert_eq!(node.log_store.last_index(), 5);
        assert_eq!(node.log_store.entry_at(4).unwrap().data, vec![4]);
        assert_eq!(node.log_store.entry_at(5).unwrap().data, vec![5]);
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
    fn bounded_read_requires_recent_successful_leader_replication() {
        let mut node = node_with_voters(&["n1", "n2"]);
        assert!(!node.can_serve_bounded_read(1_000));

        let accepted = node.handle_append_entries(
            AppendEntriesRequest {
                term: 1,
                leader_id: "n2".to_string(),
                prev_log_index: 0,
                prev_log_term: 0,
                entries: Vec::new(),
                leader_commit: 0,
            },
            200,
        );
        assert!(accepted.success);
        assert!(node.can_serve_bounded_read(1_000));

        node.set_millis(1_201);
        assert!(!node.can_serve_bounded_read(1_000));

        let rejected = node.handle_append_entries(
            AppendEntriesRequest {
                term: 1,
                leader_id: "n2".to_string(),
                prev_log_index: 99,
                prev_log_term: 1,
                entries: Vec::new(),
                leader_commit: 0,
            },
            1_202,
        );
        assert!(!rejected.success);
        assert!(!node.can_serve_bounded_read(1_000));
    }

    #[test]
    fn follower_rejects_a_conflicting_same_term_leader() {
        let mut node = node_with_voters(&["n1", "n2", "n3"]);
        let first = node.handle_append_entries(
            AppendEntriesRequest {
                term: 1,
                leader_id: "n2".to_string(),
                prev_log_index: 0,
                prev_log_term: 0,
                entries: Vec::new(),
                leader_commit: 0,
            },
            200,
        );
        assert!(first.success);
        assert_eq!(node.known_leader_id.as_deref(), Some("n2"));

        let conflicting_append = node.handle_append_entries(
            AppendEntriesRequest {
                term: 1,
                leader_id: "n3".to_string(),
                prev_log_index: 0,
                prev_log_term: 0,
                entries: Vec::new(),
                leader_commit: 0,
            },
            201,
        );
        assert!(!conflicting_append.success);
        let conflicting_snapshot = node.handle_install_snapshot(
            InstallSnapshotRequest {
                term: 1,
                leader_id: "n3".to_string(),
                last_included_index: 1,
                last_included_term: 1,
                offset: 0,
                data: Vec::new(),
                done: true,
            },
            202,
        );
        assert!(!conflicting_snapshot.success);
        assert_eq!(node.known_leader_id.as_deref(), Some("n2"));
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
        })
        .unwrap();

        assert!(node.cluster_configuration.is_joint_consensus());
        assert!(
            node.cluster_configuration
                .current_members()
                .iter()
                .any(|peer| peer.id == "n2" && peer.is_learner())
        );
        assert!(node.cluster_configuration.is_voter("n3"));
        assert_eq!(node.pending_auto_finalize_fence_index, 7);
        // Legacy envelopes without the explicit field safely fall back to
        // the index of the installed membership snapshot.
        assert_eq!(node.membership_epoch(), 7);
        assert!(!node.decommissioned);
    }
}
