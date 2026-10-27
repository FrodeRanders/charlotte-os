//! Surviving quorum over a delayed link, using the native fixture identities.
use std::sync::{
    Arc,
    Mutex,
    atomic::{
        AtomicU64,
        Ordering,
    },
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
    transport::{
        AppendEntriesRpc,
        InstallSnapshotRpc,
        RaftTransport,
    },
    types::{
        NodeState,
        Peer,
        VoteRequest,
        VoteResponse,
    },
};

const IDS: [&str; 3] = ["charlotte:42e27737", "charlotte:42e278ea", "charlotte:42e27584"];

enum Message {
    Request(VoteRequest),
    Response(VoteResponse),
}
struct Event {
    due: u64,
    source: usize,
    destination: usize,
    message: Message,
}
struct Link {
    source: usize,
    delay: u64,
    now: Arc<AtomicU64>,
    events: Arc<Mutex<Vec<Event>>>,
}
impl RaftTransport for Link {
    fn send_vote_request(
        &self,
        peer: &Peer,
        term: u64,
        candidate_id: &str,
        last_log_index: u64,
        last_log_term: u64,
    ) {
        let destination = IDS.iter().position(|id| *id == peer.id).unwrap();
        self.events.lock().unwrap().push(Event {
            due: self.now.load(Ordering::Relaxed) + self.delay,
            source: self.source,
            destination,
            message: Message::Request(VoteRequest {
                term,
                candidate_id: candidate_id.to_owned(),
                last_log_index,
                last_log_term,
            }),
        });
    }

    fn send_append_entries(&self, _: AppendEntriesRpc<'_>) {}

    fn send_install_snapshot(&self, _: InstallSnapshotRpc<'_>) {}

    fn broadcast_heartbeat_complete(&self) {}
}

fn elect(timeout: u64, delay: u64, budget: u64) -> Option<(usize, u64, u64)> {
    let now = Arc::new(AtomicU64::new(0));
    let events = Arc::new(Mutex::new(Vec::<Event>::new()));
    let peers: Vec<_> = IDS.iter().map(|id| Peer::voter((*id).to_owned(), 0)).collect();
    let mut nodes: Vec<_> = (0..2)
        .map(|source| {
            RaftNode::new(RaftNodeConfig {
                me: peers[source].clone(),
                timeout_millis: timeout,
                log_store: Box::new(InMemoryLogStore::new()),
                persistent_state: Box::new(InMemoryPersistentStateStore::new()),
                state_machine: None,
                cluster_configuration: ClusterConfiguration::stable(peers.clone()),
                transport: Arc::new(Link {
                    source,
                    delay,
                    now: now.clone(),
                    events: events.clone(),
                }),
                current_millis: 0,
                snapshot_min_entries: 64,
                snapshot_chunk_bytes: 3000,
            })
            .unwrap()
        })
        .collect();
    // The third configured voter is absent; two real survivors must exchange
    // requests and responses to elect. No manufactured vote or membership reset.
    for millis in (0..=budget).step_by(10) {
        now.store(millis, Ordering::Relaxed);
        let due: Vec<_> = {
            let mut queue = events.lock().unwrap();
            let mut due = Vec::new();
            let mut index = 0;
            while index < queue.len() {
                if queue[index].due <= millis {
                    due.push(queue.remove(index));
                } else {
                    index += 1;
                }
            }
            due
        };
        for event in due {
            let Some(node) = nodes.get_mut(event.destination) else {
                continue;
            };
            match event.message {
                Message::Request(request) => {
                    let response = node.handle_vote_request(request, millis);
                    events.lock().unwrap().push(Event {
                        due: millis + delay,
                        source: event.destination,
                        destination: event.source,
                        message: Message::Response(response),
                    });
                }
                Message::Response(response) => {
                    node.handle_vote_response(IDS[event.source], response, millis)
                }
            }
        }
        for (index, node) in nodes.iter_mut().enumerate() {
            node.set_millis(millis);
            if node.check_timeout() {
                node.start_election(millis);
            }
            if node.state == NodeState::Leader {
                return Some((index, millis, node.current_term));
            }
        }
    }
    None
}

#[test]
fn surviving_voters_elect_over_a_quarter_second_link_with_a_two_second_timeout() {
    let result = elect(2_000, 250, 20_000);
    assert!(result.is_some(), "surviving majority remained stuck in split votes");
    println!("surviving quorum elected: {result:?}");
}

#[test]
fn election_deadlines_and_repeated_backoff_do_not_wrap() {
    let peers: Vec<_> = IDS.iter().map(|id| Peer::voter((*id).to_owned(), 0)).collect();
    let mut node = RaftNode::new(RaftNodeConfig {
        me: peers[0].clone(),
        timeout_millis: u64::MAX,
        log_store: Box::new(InMemoryLogStore::new()),
        persistent_state: Box::new(InMemoryPersistentStateStore::new()),
        state_machine: None,
        cluster_configuration: ClusterConfiguration::stable(peers),
        transport: Arc::new(catten_graft::transport::NoopTransport),
        current_millis: u64::MAX - 1,
        snapshot_min_entries: 64,
        snapshot_chunk_bytes: 3000,
    })
    .unwrap();
    assert_eq!(node.timeout_at_millis, u64::MAX);
    node.election_sequence_counter = u64::MAX;
    node.start_election(u64::MAX - 1);
    assert_eq!(node.timeout_at_millis, u64::MAX);
    assert_eq!(node.election_sequence_counter, u64::MAX);
}
