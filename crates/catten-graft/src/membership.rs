use alloc::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    string::String,
    vec::Vec,
};

use crate::types::Peer;

/// Immutable, peer-ID-normalized Raft membership.
///
/// During joint consensus, quorum decisions require independent majorities
/// of both `current_members` and `next_members`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterConfiguration {
    current_members: BTreeMap<String, Peer>,
    next_members: BTreeMap<String, Peer>,
}

impl ClusterConfiguration {
    pub fn stable(members: Vec<Peer>) -> Self {
        let current_members: BTreeMap<String, Peer> =
            members.into_iter().map(|peer| (peer.id.clone(), peer)).collect();
        assert!(!current_members.is_empty(), "Raft configuration must not be empty");
        Self {
            current_members,
            next_members: BTreeMap::new(),
        }
    }

    pub fn is_joint_consensus(&self) -> bool {
        !self.next_members.is_empty()
    }

    pub fn current_members(&self) -> Vec<&Peer> {
        self.current_members.values().collect()
    }

    pub fn next_members(&self) -> Vec<&Peer> {
        self.next_members.values().collect()
    }

    pub fn current_voting_members(&self) -> Vec<&Peer> {
        self.current_members.values().filter(|peer| peer.is_voter()).collect()
    }

    pub fn next_voting_members(&self) -> Vec<&Peer> {
        if self.is_joint_consensus() {
            self.next_members.values().filter(|peer| peer.is_voter()).collect()
        } else {
            self.current_voting_members()
        }
    }

    pub fn all_members(&self) -> Vec<&Peer> {
        let mut seen = BTreeSet::new();
        let mut members = Vec::new();
        for peer in self.current_members.values().chain(self.next_members.values()) {
            if seen.insert(peer.id.as_str()) {
                members.push(peer);
            }
        }
        members
    }

    pub fn contains(&self, peer_id: &str) -> bool {
        self.current_members.contains_key(peer_id) || self.next_members.contains_key(peer_id)
    }

    pub fn is_voter(&self, peer_id: &str) -> bool {
        self.current_members.get(peer_id).is_some_and(Peer::is_voter)
            || self.next_members.get(peer_id).is_some_and(Peer::is_voter)
    }

    pub fn transition_to(&self, proposed_members: Vec<Peer>) -> Self {
        let next_members: BTreeMap<String, Peer> =
            proposed_members.into_iter().map(|peer| (peer.id.clone(), peer)).collect();
        assert!(!next_members.is_empty(), "joint Raft configuration must not be empty");
        Self {
            current_members: self.current_members.clone(),
            next_members,
        }
    }

    pub fn finalize_transition(&self) -> Self {
        if self.next_members.is_empty() {
            return self.clone();
        }
        Self {
            current_members: self.next_members.clone(),
            next_members: BTreeMap::new(),
        }
    }

    pub fn has_joint_majority(&self, peer_ids: &BTreeSet<String>) -> bool {
        has_majority(&self.current_members, peer_ids)
            && (!self.is_joint_consensus() || has_majority(&self.next_members, peer_ids))
    }
}

fn has_majority(members: &BTreeMap<String, Peer>, peer_ids: &BTreeSet<String>) -> bool {
    let voter_count = members.values().filter(|peer| peer.is_voter()).count();
    voter_count > 0
        && members.values().filter(|peer| peer.is_voter() && peer_ids.contains(&peer.id)).count()
            > voter_count / 2
}

#[cfg(test)]
mod tests {
    use alloc::{
        collections::BTreeSet,
        string::ToString,
        vec,
    };

    use super::ClusterConfiguration;
    use crate::types::Peer;

    #[test]
    fn duplicate_peer_ids_cannot_manufacture_a_quorum() {
        let configuration = ClusterConfiguration::stable(vec![
            Peer::voter("n1".to_string(), 1),
            Peer::voter("n1".to_string(), 1),
            Peer::voter("n2".to_string(), 2),
            Peer::voter("n3".to_string(), 3),
        ]);
        let only_self = BTreeSet::from(["n1".to_string()]);
        assert!(!configuration.has_joint_majority(&only_self));
        assert_eq!(configuration.all_members().len(), 3);
    }

    #[test]
    fn joint_consensus_requires_both_majorities() {
        let configuration = ClusterConfiguration::stable(vec![
            Peer::voter("a".to_string(), 1),
            Peer::voter("b".to_string(), 2),
            Peer::voter("c".to_string(), 3),
        ])
        .transition_to(vec![
            Peer::voter("c".to_string(), 3),
            Peer::voter("d".to_string(), 4),
            Peer::voter("e".to_string(), 5),
        ]);
        assert!(
            !configuration.has_joint_majority(&BTreeSet::from(["a".to_string(), "b".to_string(),]))
        );
        assert!(configuration.has_joint_majority(&BTreeSet::from([
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ])));
    }
}
