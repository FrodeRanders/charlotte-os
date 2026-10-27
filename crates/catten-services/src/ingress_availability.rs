//! Replicated admission of backends for new flows, separate from voter removal.
use alloc::vec::Vec;

use catten_graft::{
    node::RaftNode,
    types::NodeState,
};

use crate::{
    cluster_ingress::{
        Backend,
        BackendSnapshot,
        MAX_BACKENDS,
        availability_load_balancing_epoch,
        service_load_balancing_epoch,
    },
    frouter,
    name_catalog::NameCatalog,
    node_identity,
};

pub(crate) const COMMAND: u8 = 0x0f;
const HEADER_BYTES: usize = 27;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct View {
    pub term: u64,
    pub membership_epoch: u64,
    pub leader_node: u64,
    pub eligible_nodes: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub generation: u64,
    pub view: View,
}

impl View {
    pub(crate) fn encoded_len(&self) -> usize {
        HEADER_BYTES + self.eligible_nodes.len() * 8
    }

    fn valid(&self) -> bool {
        self.term != 0
            && self.leader_node != 0
            && !self.eligible_nodes.is_empty()
            && self.eligible_nodes.len() <= MAX_BACKENDS
            && self.eligible_nodes[0] != 0
            && self.eligible_nodes.windows(2).all(|pair| pair[0] < pair[1])
            && self.eligible_nodes.binary_search(&self.leader_node).is_ok()
    }

    pub(crate) fn encode(&self) -> Option<Vec<u8>> {
        if !self.valid() {
            return None;
        }
        let mut bytes = Vec::with_capacity(self.encoded_len());
        bytes.push(COMMAND);
        bytes.extend_from_slice(&self.term.to_le_bytes());
        bytes.extend_from_slice(&self.membership_epoch.to_le_bytes());
        bytes.extend_from_slice(&self.leader_node.to_le_bytes());
        bytes.extend_from_slice(&(self.eligible_nodes.len() as u16).to_le_bytes());
        for node in &self.eligible_nodes {
            bytes.extend_from_slice(&node.to_le_bytes());
        }
        Some(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HEADER_BYTES || bytes[0] != COMMAND {
            return None;
        }
        let count = usize::from(u16::from_le_bytes(bytes[25..27].try_into().ok()?));
        if count > MAX_BACKENDS || bytes.len() != HEADER_BYTES + count * 8 {
            return None;
        }
        let view = Self {
            term: u64::from_le_bytes(bytes[1..9].try_into().ok()?),
            membership_epoch: u64::from_le_bytes(bytes[9..17].try_into().ok()?),
            leader_node: u64::from_le_bytes(bytes[17..25].try_into().ok()?),
            eligible_nodes: bytes[27..]
                .as_chunks::<8>()
                .0
                .iter()
                .copied()
                .map(u64::from_le_bytes)
                .collect(),
        };
        view.valid().then_some(view)
    }
}

/// Receive-time liveness evidence, never authentication, quiescence or a lease
/// promising that a peer will remain alive. Only successful current-term
/// replication responses enter Raft's existing contact table.
pub fn candidate(node: &RaftNode) -> Option<View> {
    if node.state != NodeState::Leader
        || node.joining
        || node.decommissioned
        || !node.can_serve_bounded_read(frouter::SNAPSHOT_SOURCE_MAX_AGE_MS)
        || !node.cluster_configuration.is_voter(&node.me.id)
    {
        return None;
    }
    let mut eligible_nodes = Vec::new();
    let voters = node.cluster_configuration.active_voting_members();
    if voters.len() > MAX_BACKENDS {
        return None;
    }
    for peer in voters {
        if peer.id == node.me.id
            || node.last_follower_contact_millis.get(&peer.id).is_some_and(|last| {
                node.millis()
                    .checked_sub(*last)
                    .is_some_and(|age| age <= frouter::SNAPSHOT_SOURCE_MAX_AGE_MS)
            }) && node
                .match_index
                .get(&peer.id)
                .is_some_and(|index| node.log_store.term_at(*index) == node.current_term)
        {
            eligible_nodes.push(node_identity::key_from_name(peer.id.as_bytes())?);
        }
    }
    eligible_nodes.sort_unstable();
    let view = View {
        term: node.current_term,
        membership_epoch: node.membership_epoch(),
        leader_node: node_identity::key_from_name(node.me.id.as_bytes())?,
        eligible_nodes,
    };
    view.valid().then_some(view)
}

/// Followers use the committed projection, never their own peer timers. A
/// restored or delayed projection from another term/configuration stays fenced.
pub fn current(node: &RaftNode, catalog: &NameCatalog) -> Option<Entry> {
    if !node.can_serve_bounded_read(frouter::SNAPSHOT_SOURCE_MAX_AGE_MS) {
        return None;
    }
    let entry = catalog.ingress_availability()?;
    let leader = if node.state == NodeState::Leader {
        &node.me.id
    } else {
        node.known_leader_id.as_ref()?
    };
    if entry.view.term != node.current_term
        || entry.view.membership_epoch != node.membership_epoch()
        || node_identity::key_from_name(leader.as_bytes()) != Some(entry.view.leader_node)
        || entry.view.eligible_nodes.iter().any(|key| {
            !node
                .cluster_configuration
                .active_voting_members()
                .iter()
                .any(|peer| node_identity::key_from_name(peer.id.as_bytes()) == Some(*key))
        })
    {
        return None;
    }
    Some(entry)
}

/// One accepted metadata proposal per leadership episode. It is not an RPC or
/// physical retry owner. Changes observed while it is pending wait for that
/// exact log entry; a new leader derives its own separately fenced projection.
#[derive(Default)]
pub struct Publisher {
    pending: Option<PendingProjection>,
}

struct PendingProjection {
    index: u64,
    expected: Entry,
}

impl Publisher {
    pub fn drive(
        &mut self,
        node: &mut RaftNode,
        catalog: &NameCatalog,
    ) -> Result<(), &'static str> {
        if let Some(pending) = self.pending.as_ref() {
            if node.state != NodeState::Leader || pending.expected.view.term != node.current_term {
                self.pending = None;
            } else if !node.is_committed(pending.index) {
                return Ok(());
            } else {
                // The current exact record outlives Raft's bounded receipt
                // cache. Retain the expected generation/view until this index
                // commits; missing receipts never authorize metadata replay.
                if catalog.ingress_availability().as_ref() != Some(&pending.expected) {
                    return Err("availability commit rejected");
                }
                self.pending = None;
            }
        }
        let Some(view) = candidate(node) else {
            return Ok(());
        };
        let previous = catalog.ingress_availability();
        if previous.as_ref().is_some_and(|entry| entry.view == view) {
            return Ok(());
        }
        let command = view.encode().ok_or("invalid availability candidate")?;
        let generation = previous
            .as_ref()
            .map_or(Some(1), |entry| entry.generation.checked_add(1))
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or("availability generation exhausted")?;
        if let Ok(index) = node.submit_command(command, node.millis()) {
            self.pending = Some(PendingProjection {
                index,
                expected: Entry {
                    generation,
                    view,
                },
            });
        }
        Ok(())
    }
}

/// Materialize one all-or-nothing view of committed ingress eligibility.
/// Discovery supplies only the Ethernet route for each already-admitted
/// identity. If any route is missing, callers retain their previous snapshot
/// instead of selecting from inconsistent partial sets on different nodes.
pub fn snapshot(
    node: &RaftNode,
    catalog: &NameCatalog,
    local_mac: [u8; 6],
    service_name: Option<&[u8]>,
    mut peer_mac: impl FnMut(&str) -> Option<[u8; 6]>,
) -> Option<BackendSnapshot> {
    let availability = current(node, catalog)?;
    let self_node = node_identity::key_from_name(node.me.id.as_bytes())?;
    let mut members = Vec::new();
    for peer in node.cluster_configuration.active_voting_members() {
        let node_id = node_identity::key_from_name(peer.id.as_bytes())?;
        let mac = if peer.id == node.me.id {
            local_mac
        } else {
            peer_mac(&peer.id)?
        };
        members.push(Backend {
            node_id,
            mac,
        });
    }
    let mut draining = catalog
        .ingress_draining_nodes()
        .into_iter()
        .filter(|(node_id, _)| members.iter().any(|member| member.node_id == *node_id))
        .collect::<Vec<_>>();
    draining.sort_unstable_by_key(|(node_id, _)| *node_id);
    let ingress_nodes = members
        .iter()
        .filter(|member| {
            draining.binary_search_by_key(&member.node_id, |(node_id, _)| *node_id).is_err()
                && availability.view.eligible_nodes.binary_search(&member.node_id).is_ok()
        })
        .map(|member| member.node_id)
        .collect::<Vec<_>>();
    let (deployment_generation, service_generation, placed_nodes) = service_name.map_or_else(
        || (0, 0, None),
        |name| {
            catalog.ingress_placement(name).map_or((0, 0, Some(Vec::new())), |placement| {
                (
                    placement.deployment_generation,
                    placement.service_generation,
                    Some(placement.ready_nodes),
                )
            })
        },
    );
    let eligible_nodes = placed_nodes.map_or_else(
        || ingress_nodes.clone(),
        |placed| {
            ingress_nodes
                .iter()
                .copied()
                .filter(|node_id| placed.contains(node_id))
                .collect::<Vec<_>>()
        },
    );
    let leader = if node.state == NodeState::Leader {
        Some(node.me.id.as_str())
    } else {
        node.known_leader_id.as_deref()
    };
    let advertiser_node = leader
        .filter(|leader| {
            node.cluster_configuration
                .active_voting_members()
                .iter()
                .any(|peer| peer.id.as_str() == *leader)
        })
        .and_then(|leader| node_identity::key_from_name(leader.as_bytes()))
        .filter(|leader| ingress_nodes.contains(leader))
        .or_else(|| ingress_nodes.iter().copied().min())
        .filter(|_| !eligible_nodes.is_empty());
    let assignment_sequence = catalog
        .ingress_policy()
        .and_then(|entry| {
            charlotte_launch::ingress_policy::decode(&entry.envelope).map(|policy| policy.sequence)
        })
        .unwrap_or(0);
    let epoch = service_load_balancing_epoch(
        node.membership_epoch(),
        service_name.unwrap_or(b""),
        assignment_sequence,
        deployment_generation,
        service_generation,
        &eligible_nodes,
        &draining,
    );
    let epoch = availability_load_balancing_epoch(epoch, availability.generation);
    BackendSnapshot::new_with_members(epoch, self_node, advertiser_node, members, eligible_nodes)
}
