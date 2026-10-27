use alloc::{
    boxed::Box,
    sync::Arc,
    vec,
};

use catten_graft::{
    log_store::{
        InMemoryLogStore,
        InMemoryPersistentStateStore,
    },
    membership::ClusterConfiguration,
    node::RaftNodeConfig,
    state_machine::{
        SnapshotError,
        StateMachine,
    },
    transport::NoopTransport,
    types::{
        AppendEntriesRequest,
        Peer,
        VoteRequest,
    },
};

use super::*;
use crate::cluster_ingress::{
    FlowEpochTable,
    FlowKey,
    ParsedFlow,
    ServiceId,
    SnapshotHistory,
    select_backend,
    snapshot_for_flow,
};

const IDS: [&str; 3] = ["n:00000001", "n:00000002", "n:00000003"];
const SERVICE: ServiceId = ServiceId {
    address: [10, 0, 2, 42],
    port: 443,
    protocol: 6,
};
fn view(nodes: &[u64]) -> View {
    View {
        term: 1,
        membership_epoch: 0,
        leader_node: 1,
        eligible_nodes: nodes.to_vec(),
    }
}
fn catalog() -> Arc<NameCatalog> {
    NameCatalog::new()
}
struct Machine(Arc<NameCatalog>);
impl StateMachine for Machine {
    fn apply(&self, term: u64, command: &[u8]) {
        self.0.apply(term, command);
    }

    fn apply_with_result(&self, term: u64, command: &[u8]) -> Vec<u8> {
        self.0.apply_with_result(term, command)
    }

    fn reset(&self) {
        self.0.reset();
    }

    fn snapshot(&self) -> Vec<u8> {
        self.0.snapshot()
    }

    fn restore(&self, bytes: &[u8]) -> Result<(), SnapshotError> {
        self.0.restore(bytes)
    }
}
fn node(index: usize, catalog: Arc<NameCatalog>) -> RaftNode {
    RaftNode::new(RaftNodeConfig {
        me: Peer::voter(IDS[index].into(), 0),
        timeout_millis: 2000,
        log_store: Box::new(InMemoryLogStore::new()),
        persistent_state: Box::new(InMemoryPersistentStateStore::new()),
        state_machine: Some(Box::new(Machine(catalog))),
        cluster_configuration: ClusterConfiguration::stable(
            IDS.iter().map(|id| Peer::voter((*id).into(), 0)).collect(),
        ),
        transport: Arc::new(NoopTransport),
        current_millis: 0,
        snapshot_min_entries: 128,
        snapshot_chunk_bytes: 3000,
    })
    .unwrap()
}
fn replicate(leader: &mut RaftNode, follower: &mut RaftNode, now: u64) {
    let reply = follower.handle_append_entries(
        AppendEntriesRequest {
            term: leader.current_term,
            leader_id: leader.me.id.clone(),
            prev_log_index: 0,
            prev_log_term: 0,
            leader_commit: leader.commit_index,
            entries: leader.log_store.entries_from(1),
        },
        now,
    );
    assert!(reply.success);
    leader.handle_append_entries_response(&follower.me.id, reply, now);
}
fn mac(id: &str) -> Option<[u8; 6]> {
    Some([2, 0, 0, 0, 0, node_identity::key_from_name(id.as_bytes())? as u8])
}
fn project(node: &RaftNode, catalog: &NameCatalog) -> Option<BackendSnapshot> {
    snapshot(node, catalog, mac(&node.me.id)?, None, mac)
}
fn flow(port: u16) -> ParsedFlow {
    ParsedFlow {
        key: FlowKey {
            protocol: 6,
            src_addr: [10, 0, 2, 2],
            dst_addr: SERVICE.address,
            src_port: port,
            dst_port: 443,
        },
        initial_syn: true,
        terminal: false,
    }
}

#[test]
fn quorum_commit_precedes_eligibility_and_recovery_does_not_replay_bound_flows() {
    let ca = catalog();
    let cb = catalog();
    let cc = catalog();
    let mut a = node(0, ca.clone());
    let mut b = node(1, cb.clone());
    let mut c = node(2, cc);
    a.start_election(0);
    let vote = b.handle_vote_request(
        VoteRequest {
            term: a.current_term,
            candidate_id: a.me.id.clone(),
            last_log_index: 0,
            last_log_term: 0,
        },
        10,
    );
    a.handle_vote_response(&b.me.id, vote, 10);
    assert_eq!(a.state, NodeState::Leader);
    assert!(candidate(&a).is_none()); // No committed current-term prefix yet.
    replicate(&mut a, &mut b, 100);
    replicate(&mut a, &mut c, 101);
    let mut publisher = Publisher::default();
    publisher.drive(&mut a, &ca).unwrap();
    assert!(ca.ingress_availability().is_none());
    assert!(project(&a, &ca).is_none());
    replicate(&mut a, &mut b, 110);
    replicate(&mut a, &mut b, 111);
    replicate(&mut a, &mut c, 112);
    a.applied_command_results.clear(); // Receipt eviction cannot authorize replay.
    publisher.drive(&mut a, &ca).unwrap();
    let before = project(&a, &ca).unwrap();
    assert_eq!(before.backends().len(), 3);
    let original = (50000..51000)
        .map(flow)
        .find(|p| select_backend(&SERVICE, &p.key, &before).unwrap().node_id == 3)
        .unwrap();
    let mut history = SnapshotHistory::new(4);
    history.install(before.clone());
    let mut flows = FlowEpochTable::new(8);
    snapshot_for_flow(&original, &history, &mut flows, true).unwrap();
    // Only B responds now. C's current-term contact has expired, while a real
    // surviving quorum remains fresh; its voter and Ethernet identity remain.
    replicate(&mut a, &mut b, 1200);
    publisher.drive(&mut a, &ca).unwrap();
    let pending_index = a.log_store.last_index();
    publisher.drive(&mut a, &ca).unwrap();
    assert_eq!(a.log_store.last_index(), pending_index);
    assert_eq!(project(&a, &ca).unwrap().backends().len(), 3);
    assert_eq!(project(&b, &cb).unwrap().backends().len(), 3); // No independent follower expiry.
    replicate(&mut a, &mut b, 1201);
    replicate(&mut a, &mut b, 1202);
    publisher.drive(&mut a, &ca).unwrap();
    let after = project(&a, &ca).unwrap();
    let follower = project(&b, &cb).unwrap();
    assert_eq!(after.members().len(), 3);
    assert_eq!(after.backends().iter().map(|n| n.node_id).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(after.epoch, follower.epoch);
    assert_eq!(after.backends(), follower.backends());
    assert_eq!(after.vip_advertiser(), follower.vip_advertiser());
    history.install(after.clone());
    assert_eq!(flows.remove_absent_backends(&SERVICE, &history, &after), 0);
    let pinned = snapshot_for_flow(&original, &history, &mut flows, true).unwrap();
    assert_eq!(pinned.epoch, before.epoch);
    assert_eq!(select_backend(&SERVICE, &original.key, pinned).unwrap().node_id, 3);
    let next = (51000..52000)
        .map(flow)
        .find(|p| select_backend(&SERVICE, &p.key, &before).unwrap().node_id == 3)
        .unwrap();
    let fresh = snapshot_for_flow(&next, &history, &mut flows, true).unwrap();
    assert_eq!(fresh.epoch, after.epoch);
    assert_ne!(select_backend(&SERVICE, &next.key, fresh).unwrap().node_id, 3);
    // A later real replication acknowledgement can re-admit C only after a
    // fresh committed projection; recovering the same set has a new epoch.
    replicate(&mut a, &mut c, 1300);
    publisher.drive(&mut a, &ca).unwrap();
    assert_eq!(project(&a, &ca).unwrap().backends().len(), 2);
    replicate(&mut a, &mut b, 1301);
    publisher.drive(&mut a, &ca).unwrap();
    let recovered = project(&a, &ca).unwrap();
    assert_eq!(recovered.backends().len(), 3);
    assert_ne!(before.epoch, recovered.epoch);
    assert_eq!(ca.ingress_availability().unwrap().generation, 3);
    a.membership_epoch += 1;
    assert!(current(&a, &ca).is_none());
    a.membership_epoch -= 1;
    a.current_term += 1;
    assert!(current(&a, &ca).is_none());
    assert!(candidate(&a).is_none());
}

#[test]
fn complete_canonical_codec_rejects_malformed_projection_without_prefixes() {
    let good = view(&[1, 2, 3]);
    let bytes = good.encode().unwrap();
    assert_eq!(View::decode(&bytes), Some(good));
    for end in 0..bytes.len() {
        assert!(View::decode(&bytes[..end]).is_none());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(View::decode(&extra).is_none());
    for nodes in [vec![], vec![1, 1], vec![2, 1], vec![0, 1], vec![2, 3], (1..=65).collect()] {
        assert!(view(&nodes).encode().is_none());
    }
    let mut wrong = bytes.clone();
    wrong[0] = 0x0e;
    assert!(View::decode(&wrong).is_none());
    let mut zero = bytes;
    zero[1..9].fill(0);
    assert!(View::decode(&zero).is_none());
}

#[test]
fn catalog_fences_term_leader_and_membership_and_snapshots_restore_atomically() {
    let source = catalog();
    let first = view(&[1, 2, 3]);
    let command = first.encode().unwrap();
    assert!(source.apply_with_result(2, &command).is_empty());
    assert!(source.ingress_availability().is_none());
    assert_eq!(source.apply_with_result(1, &command), 1i64.to_le_bytes());
    assert_eq!(source.apply_with_result(1, &command), 1i64.to_le_bytes());
    let smaller = view(&[1, 2]);
    assert_eq!(source.apply_with_result(1, &smaller.encode().unwrap()), 2i64.to_le_bytes());
    let target = catalog();
    target.apply(1, &crate::name_catalog::encode_register(b"live", b"n:00000001", 0));
    let before = target.snapshot();
    let snapshot = source.snapshot();
    let section = snapshot.len() - (1 + 8 + 4 + smaller.encoded_len());
    for (position, value) in [(section, 2), (section + 1, 0), (section + 13 + 25, 65)] {
        let mut corrupt = snapshot.clone();
        corrupt[position] = value;
        assert!(target.restore(&corrupt).is_err());
        assert_eq!(target.snapshot(), before);
    }
    let mut exhausted = snapshot.clone();
    exhausted[section + 1..section + 9].copy_from_slice(&(i64::MAX as u64).to_le_bytes());
    target.restore(&exhausted).unwrap();
    assert!(target.apply_with_result(1, &command).is_empty());
    assert_eq!(target.snapshot(), exhausted);
    target.restore(&before).unwrap();
    for end in 0..snapshot.len() {
        assert!(target.restore(&snapshot[..end]).is_err());
        assert_eq!(target.snapshot(), before);
    }
    let mut extra = snapshot.clone();
    extra.push(0);
    assert!(target.restore(&extra).is_err());
    assert_eq!(target.snapshot(), before);
    let mut old = snapshot.clone();
    old[..8].copy_from_slice(&0x4341_5441_4c4f_4749u64.to_le_bytes());
    assert_eq!(target.restore(&old), Err(SnapshotError::Unsupported));
    assert_eq!(target.snapshot(), before);
    target.restore(&snapshot).unwrap();
    assert_eq!(target.ingress_availability(), source.ingress_availability());
    assert_eq!(target.snapshot(), snapshot);
    let mut newer = smaller.clone();
    newer.term = 2;
    newer.membership_epoch = 4;
    assert_eq!(target.apply_with_result(2, &newer.encode().unwrap()), 3i64.to_le_bytes());
    let before = target.snapshot();
    let mut other_leader = newer.clone();
    other_leader.leader_node = 2;
    let mut stale = newer;
    stale.membership_epoch = 3;
    for rejected in [smaller, other_leader, stale] {
        assert!(target.apply_with_result(rejected.term, &rejected.encode().unwrap()).is_empty());
        assert_eq!(target.snapshot(), before);
    }
    target.reset();
    assert!(target.ingress_availability().is_none());
}
