//! Replicated name catalog: the Raft state machine behind the distributed
//! name service.
//!
//! Maps `name -> {node_id, generation}`. Connections are
//! node-local capabilities and cannot be replicated, so only the *location* of
//! a registration is committed; resolving it to a connection stays a local
//! operation on the hosting node.
//!
//! The same state machine also carries the cluster **deployment manifest**
//! (`artifact -> {object_id, artifact_sha256, replica_nodes, descriptor, generation}`) and the
//! cluster's **Ed25519 public key** (committed by the key ceremony). A deployment is a
//! cluster decision, so it lives in replicated state next to the name
//! catalog; node-local agents read it and act on the assignments addressed
//! to them. The deployment record itself needs no signature: its
//! authenticity comes from the Raft consensus that committed it.
//!
//! ## Log command encoding
//!
//! ```text
//! register:   0x01 | name_len:u32 | name | node_len:u32 | node
//! unregister: 0x02 | name_len:u32 | name
//! deploy:     0x05 | artifact_len:u32 | artifact | object_id:u64 | node_key:u64 | sha256:32 |
//!             descriptor_len:u32 | signed_descriptor
//! set-key:    0x07 | key:[u8; 32]
//! release:    0x08 | envelope_len:u32 | signed_release | node_count:u16 | node_keys:[u64]
//!             [bundle_sequence:u64 | bundle_sha256:32 | binding_count:u16 | compact_bindings]
//! replicas:   0x0a | envelope_len:u32 | signed_release | descriptor_count:u16 |
//!             [replica_count:u16 | node_keys:[u64]] [optional operational tail]
//! reassign:   0x0b | artifact_len:u32 | artifact | expected_generation:u64 |
//!             replica_count:u16 | node_keys:[u64]
//! ingress:    0x0c | envelope_len:u32 | signed_ingress_policy
//! capacity:   0x0d | node_key:u64 | boot_nonce:u64 | epoch:u64 |
//!             free_frames:u64 | usable_frames:u64 | cpu_load_permille:u16
//! ```
use alloc::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    sync::Arc,
    vec::Vec,
};

use catten_graft::state_machine::{
    QueryableStateMachine,
    StateMachine,
};

mod snapshot;

#[cfg(test)]
mod tests;

const CMD_REGISTER: u8 = 0x01;
const CMD_UNREGISTER: u8 = 0x02;
const CMD_ACTIVATE: u8 = 0x03;
const CMD_UNREGISTER_GENERATION: u8 = 0x04;
const CMD_DEPLOY: u8 = 0x05;
const CMD_SET_CLUSTER_KEY: u8 = 0x07;
const CMD_RELEASE: u8 = 0x08;
const CMD_SHUTDOWN: u8 = 0x09;
const CMD_RELEASE_REPLICAS: u8 = 0x0a;
const CMD_REASSIGN: u8 = 0x0b;
const CMD_INGRESS_POLICY: u8 = 0x0c;
const CMD_NODE_CAPACITY: u8 = 0x0d;

/// Bounds on the replicated record collections. Tombstones are monotonic by
/// design, so a cap applies only to new keys; replacing an existing entry
/// always succeeds.
const MAX_ENTRIES: usize = 4_096;
const MAX_DEPLOYMENTS: usize = 4_096;
const MAX_RELEASES: usize = 4_096;
const MAX_OPERATIONAL_BINDINGS: usize = 8_192;
const MAX_SHUTDOWN_INTENTS: usize = 4_096;
const MAX_NODE_CAPACITY_ENTRIES: usize = 256;
const CATALOG_MAGIC_V1: u64 = 0x4341_5441_4c4f_474d; // "CATALOGM"
const CATALOG_MAGIC_V2: u64 = 0x4341_5441_4c4f_4732; // "CATALOG2"
const CATALOG_MAGIC_V3: u64 = 0x4341_5441_4c4f_4733; // "CATALOG3"
const CATALOG_MAGIC_V4: u64 = 0x4341_5441_4c4f_4734; // "CATALOG4"
const CATALOG_MAGIC_V5: u64 = 0x4341_5441_4c4f_4735; // "CATALOG5"
const CATALOG_MAGIC_V6: u64 = 0x4341_5441_4c4f_4736; // "CATALOG6"
const CATALOG_MAGIC_V7: u64 = 0x4341_5441_4c4f_4737; // "CATALOG7"
const CATALOG_MAGIC_V8: u64 = 0x4341_5441_4c4f_4738; // "CATALOG8"
const CATALOG_MAGIC_V9: u64 = 0x4341_5441_4c4f_4739; // "CATALOG9"
const CATALOG_MAGIC_V10: u64 = 0x4341_5441_4c4f_4741; // "CATALOGA"
const CATALOG_MAGIC_V11: u64 = 0x4341_5441_4c4f_4742; // "CATALOGB"
const CATALOG_MAGIC_V12: u64 = 0x4341_5441_4c4f_4743; // "CATALOGC"
const CATALOG_MAGIC_V13: u64 = 0x4341_5441_4c4f_4744; // "CATALOGD"
const CATALOG_MAGIC_V14: u64 = 0x4341_5441_4c4f_4745; // "CATALOGE"
const CATALOG_MAGIC_V15: u64 = 0x4341_5441_4c4f_4746; // "CATALOGF"

/// Query tag prefix for a name lookup.
const QUERY_LOOKUP: u8 = 0x01;
/// Query tag prefix for a deployment query.
const QUERY_DEPLOY: u8 = 0x02;
/// Query tag prefix for the latest shutdown intent targeting one node.
const QUERY_SHUTDOWN: u8 = 0x03;
/// Query the current cluster ingress assignment policy.
const QUERY_INGRESS_POLICY: u8 = 0x04;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    pub node: Vec<u8>,
    pub generation: u64,
    pub active: bool,
    /// Desired deployment generation whose application endpoint produced
    /// this registration. Zero denotes an ordinary system service.
    pub deployment_generation: u64,
}

/// A replicated deployment record: the cluster's answer to "which nodes run
/// this artifact, and from which object-store object?".
///
/// `node_key` is the packed cluster node identity (the FNV-1a of the node's
/// NIC MAC). The record needs no signature: the Raft consensus that
/// committed it is its authenticity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentEntry {
    pub object_id: u64,
    /// Compatibility/diagnostic primary: the first member of
    /// `replica_nodes`, or zero when no placement is currently possible.
    pub node_key: u64,
    /// Sorted, unique concrete assignments committed by Raft.
    pub replica_nodes: Vec<u64>,
    pub generation: u64,
    /// Immutable content identity selected by this deployment generation.
    pub artifact_digest: [u8; 32],
    /// Signed, bounded deployment decision. Empty only for a legacy record.
    pub descriptor: Vec<u8>,
}

/// The committed placement/readiness view used to admit new ingress flows.
///
/// Each ready node is both a member of the committed replica set and the
/// publisher of an endpoint for the exact deployment generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngressPlacement {
    pub deployment_generation: u64,
    pub service_generation: u64,
    pub ready_nodes: Vec<u64>,
}

/// One atomically admitted, signed component set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseEntry {
    pub generation: u64,
    pub envelope: Vec<u8>,
    /// Zero for a release admitted without an operational bundle.
    pub operations_sequence: u64,
    pub operations_bundle_digest: [u8; 32],
}

/// Compact replicated reference to one encrypted connector profile. The
/// ciphertext remains in the central object store and is rechecked against
/// `envelope_digest` before node-local decryption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationalBindingEntry {
    pub generation: u64,
    pub active: bool,
    pub release_name: Vec<u8>,
    pub release_digest: [u8; 32],
    pub bundle_sequence: u64,
    pub bundle_digest: [u8; 32],
    pub target_artifact: Vec<u8>,
    pub object_key: Vec<u8>,
    pub envelope_digest: [u8; 32],
    pub profile_kind: u16,
    pub sequence: u64,
    pub expires_unix_seconds: u64,
    pub recipient_key_id: [u8; charlotte_launch::operations::KEY_ID_LEN],
    pub signing_key_id: [u8; charlotte_launch::operations::KEY_ID_LEN],
    pub authorization_signature: [u8; charlotte_launch::operations_bundle::BINDING_SIGNATURE_LEN],
}

/// Latest replicated, operator-signed shutdown intent for one node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShutdownIntentEntry {
    pub generation: u64,
    pub envelope: Vec<u8>,
}

/// Latest replicated, operator-signed complete ingress assignment table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngressPolicyEntry {
    pub generation: u64,
    pub envelope: Vec<u8>,
}

/// Latest committed capacity sample for one node.
///
/// The sample is written by the leader from an advisory `rcapacity` report
/// and replicated through the log so every replica resolves placements from
/// applied state. `boot_nonce` is generated once per reporter start, so a
/// restart with a reset monotonic clock supersedes the previous boot's
/// samples instead of being fenced out as stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeCapacityEntry {
    pub node_key: u64,
    pub boot_nonce: u64,
    pub epoch: u64,
    pub free_frames: u64,
    pub usable_frames: u64,
    pub cpu_load_permille: u16,
}

type DeploymentReplicaMap = BTreeMap<Vec<u8>, BTreeMap<Vec<u8>, CatalogEntry>>;

pub struct NameCatalog {
    entries: spin::Mutex<BTreeMap<Vec<u8>, CatalogEntry>>,
    /// Per-node publications for replicated deployed applications. Ordinary
    /// service names retain the singleton `entries` representation.
    deployment_replicas: spin::Mutex<DeploymentReplicaMap>,
    deployments: spin::Mutex<BTreeMap<Vec<u8>, DeploymentEntry>>,
    releases: spin::Mutex<BTreeMap<Vec<u8>, ReleaseEntry>>,
    operational_bindings: spin::Mutex<BTreeMap<Vec<u8>, OperationalBindingEntry>>,
    shutdown_intents: spin::Mutex<BTreeMap<u64, ShutdownIntentEntry>>,
    node_capacity: spin::Mutex<BTreeMap<u64, NodeCapacityEntry>>,
    ingress_policy: spin::Mutex<Option<IngressPolicyEntry>>,
    cluster_key: spin::Mutex<Option<[u8; 32]>>,
    cluster_key_generation: spin::Mutex<u64>,
    /// Launch-owned deployment/release authority used before an optional
    /// replicated key ceremony commits a replacement.
    bootstrap_deployment_key: [u8; 32],
    bootstrap_operations_key: [u8; 32],
    cluster_id: [u8; 32],
    last_apply: spin::Mutex<Option<Vec<u8>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompactOperationalBinding {
    target_artifact: Vec<u8>,
    profile_name: Vec<u8>,
    object_key: Vec<u8>,
    envelope_digest: [u8; 32],
    profile_kind: u16,
    sequence: u64,
    expires_unix_seconds: u64,
    recipient_key_id: [u8; charlotte_launch::operations::KEY_ID_LEN],
    signing_key_id: [u8; charlotte_launch::operations::KEY_ID_LEN],
    authorization_signature: [u8; charlotte_launch::operations_bundle::BINDING_SIGNATURE_LEN],
}

struct CompactOperationalSet {
    bundle_sequence: u64,
    bundle_digest: [u8; 32],
    bindings: Vec<CompactOperationalBinding>,
}

type ParsedOperationalTail = Option<CompactOperationalSet>;

fn release_contains_artifact(
    release: &charlotte_launch::release::ReleaseEnvelope<'_>,
    target: &[u8],
) -> bool {
    release.descriptors().any(|bytes| {
        charlotte_launch::deployment::decode(bytes)
            .is_some_and(|descriptor| descriptor.artifact_name == target)
    })
}

fn parse_operational_tail(
    command: &[u8],
    offset: usize,
    release: &charlotte_launch::release::ReleaseEnvelope<'_>,
) -> Option<ParsedOperationalTail> {
    if offset == command.len() {
        return Some(None);
    }
    let (bundle_sequence, after_sequence) = read_u64(command, offset)?;
    let bundle_digest: [u8; 32] =
        command.get(after_sequence..after_sequence + 32)?.try_into().ok()?;
    let (binding_count, mut position) = read_u16(command, after_sequence + 32)?;
    if bundle_sequence == 0
        || bundle_digest.iter().all(|byte| *byte == 0)
        || binding_count == 0
        || usize::from(binding_count) > charlotte_launch::operations_bundle::MAX_BINDINGS
    {
        return None;
    }
    let mut bindings = Vec::with_capacity(usize::from(binding_count));
    for _ in 0..binding_count {
        let (sequence, after_binding_sequence) = read_u64(command, position)?;
        let (expires_unix_seconds, after_expiry) = read_u64(command, after_binding_sequence)?;
        let (profile_kind, after_kind) = read_u16(command, after_expiry)?;
        let (target_len, after_target_len) = read_u16(command, after_kind)?;
        let (profile_len, after_profile_len) = read_u16(command, after_target_len)?;
        let (object_len, after_object_len) = read_u16(command, after_profile_len)?;
        let envelope_digest: [u8; 32] =
            command.get(after_object_len..after_object_len + 32)?.try_into().ok()?;
        let recipient_key_id: [u8; charlotte_launch::operations::KEY_ID_LEN] =
            command.get(after_object_len + 32..after_object_len + 48)?.try_into().ok()?;
        let signing_key_id: [u8; charlotte_launch::operations::KEY_ID_LEN] =
            command.get(after_object_len + 48..after_object_len + 64)?.try_into().ok()?;
        let authorization_signature: [u8;
            charlotte_launch::operations_bundle::BINDING_SIGNATURE_LEN] =
            command.get(after_object_len + 64..after_object_len + 128)?.try_into().ok()?;
        let target_start = after_object_len + 128;
        let target_end = target_start.checked_add(usize::from(target_len))?;
        let profile_end = target_end.checked_add(usize::from(profile_len))?;
        let object_end = profile_end.checked_add(usize::from(object_len))?;
        let target_artifact = command.get(target_start..target_end)?;
        let profile_name = command.get(target_end..profile_end)?;
        let object_key = command.get(profile_end..object_end)?;
        if sequence == 0
            || expires_unix_seconds == 0
            || !charlotte_launch::operations::valid_profile_kind(profile_kind)
            || !charlotte_launch::deployment::valid_artifact_name(target_artifact)
            || !release_contains_artifact(release, target_artifact)
            || !charlotte_launch::operations::valid_profile_name(profile_name)
            || !charlotte_launch::operations_bundle::valid_object_key(object_key)
            || envelope_digest.iter().all(|byte| *byte == 0)
            || recipient_key_id.iter().all(|byte| *byte == 0)
            || signing_key_id.iter().all(|byte| *byte == 0)
            || authorization_signature.iter().all(|byte| *byte == 0)
            || bindings.iter().any(|binding: &CompactOperationalBinding| {
                binding.target_artifact == target_artifact
                    || binding.profile_name == profile_name
                    || binding.object_key == object_key
            })
        {
            return None;
        }
        bindings.push(CompactOperationalBinding {
            target_artifact: target_artifact.to_vec(),
            profile_name: profile_name.to_vec(),
            object_key: object_key.to_vec(),
            envelope_digest,
            profile_kind,
            sequence,
            expires_unix_seconds,
            recipient_key_id,
            signing_key_id,
            authorization_signature,
        });
        position = object_end;
    }
    (position == command.len()).then_some(Some(CompactOperationalSet {
        bundle_sequence,
        bundle_digest,
        bindings,
    }))
}

impl NameCatalog {
    pub fn new() -> Arc<Self> {
        Self::new_with_deployment_key(charlotte_launch::CLUSTER_PUBLIC_KEY)
    }

    pub fn new_with_deployment_key(bootstrap_deployment_key: [u8; 32]) -> Arc<Self> {
        Self::new_with_control_plane_trust(
            bootstrap_deployment_key,
            bootstrap_deployment_key,
            charlotte_launch::trust::cluster_id(b"charlotte").expect("static cluster name"),
        )
    }

    pub fn new_with_control_plane_trust(
        bootstrap_deployment_key: [u8; 32],
        bootstrap_operations_key: [u8; 32],
        cluster_id: [u8; 32],
    ) -> Arc<Self> {
        assert!(bootstrap_deployment_key.iter().any(|byte| *byte != 0));
        assert!(bootstrap_operations_key.iter().any(|byte| *byte != 0));
        assert!(cluster_id.iter().any(|byte| *byte != 0));
        Arc::new(Self {
            entries: spin::Mutex::new(BTreeMap::new()),
            deployment_replicas: spin::Mutex::new(BTreeMap::new()),
            deployments: spin::Mutex::new(BTreeMap::new()),
            releases: spin::Mutex::new(BTreeMap::new()),
            operational_bindings: spin::Mutex::new(BTreeMap::new()),
            shutdown_intents: spin::Mutex::new(BTreeMap::new()),
            node_capacity: spin::Mutex::new(BTreeMap::new()),
            ingress_policy: spin::Mutex::new(None),
            cluster_key: spin::Mutex::new(None),
            cluster_key_generation: spin::Mutex::new(0),
            bootstrap_deployment_key,
            bootstrap_operations_key,
            cluster_id,
            last_apply: spin::Mutex::new(None),
        })
    }

    /// The replicated owner and service generation for `name`, or `None`.
    pub fn lookup(&self, name: &[u8]) -> Option<CatalogEntry> {
        let desired = self.deployment(name);
        if let Some(entry) = self
            .deployment_replicas
            .lock()
            .get(name)
            .and_then(|entries| {
                entries.values().find(|entry| {
                    let node = crate::node_identity::key_from_name(&entry.node);
                    entry.active
                        && !entry.node.is_empty()
                        && desired.as_ref().is_some_and(|deployment| {
                            entry.deployment_generation == deployment.generation
                                && node.is_some_and(|node| deployment.replica_nodes.contains(&node))
                        })
                })
            })
            .cloned()
        {
            return Some(entry);
        }
        let entry = self
            .entries
            .lock()
            .get(name)
            .filter(|entry| entry.active && !entry.node.is_empty())
            .cloned()?;
        if entry.deployment_generation == 0 {
            return Some(entry);
        }
        let deployment = desired?;
        let node = crate::node_identity::key_from_name(&entry.node)?;
        (entry.deployment_generation == deployment.generation
            && deployment.replica_nodes.contains(&node))
        .then_some(entry)
    }

    /// Resolve one exact active owner. This is the fencing primitive used by
    /// publication teardown when a deployed name has several replicas.
    pub fn lookup_owner(&self, name: &[u8], node: &[u8]) -> Option<CatalogEntry> {
        self.entries
            .lock()
            .get(name)
            .filter(|entry| entry.active && entry.node == node)
            .cloned()
            .or_else(|| {
                self.deployment_replicas
                    .lock()
                    .get(name)
                    .and_then(|entries| entries.get(node))
                    .filter(|entry| entry.active && entry.node == node)
                    .cloned()
            })
    }

    /// The replicated deployment record for `artifact`, or `None`.
    pub fn deployment(&self, artifact: &[u8]) -> Option<DeploymentEntry> {
        self.deployments.lock().get(artifact).cloned()
    }

    /// Snapshot copy of every desired deployment, sorted by artifact name.
    pub fn deployments(&self) -> Vec<(Vec<u8>, DeploymentEntry)> {
        self.deployments.lock().iter().map(|(name, entry)| (name.clone(), entry.clone())).collect()
    }

    /// Resolve the exact committed deployment generation to nodes that have
    /// published matching, active application endpoints.
    ///
    /// A stale endpoint from the previous placement is deliberately not
    /// ready for new ingress flows. Its node may remain in an older ingress
    /// snapshot so established TCP flows can drain without making the stale
    /// registration eligible for new connections.
    pub fn ingress_placement(&self, artifact: &[u8]) -> Option<IngressPlacement> {
        let deployment = self.deployment(artifact)?;
        let entries = self.entries.lock();
        let replicas = self.deployment_replicas.lock();
        let mut service_generation = 0;
        let mut ready_nodes = Vec::new();
        if let Some(entry) = entries.get(artifact) {
            service_generation = service_generation.max(entry.generation);
            if entry.active
                && entry.deployment_generation == deployment.generation
                && let Some(node) = crate::node_identity::key_from_name(&entry.node)
                && deployment.replica_nodes.contains(&node)
            {
                ready_nodes.push(node);
            }
        }
        if let Some(by_node) = replicas.get(artifact) {
            for entry in by_node.values() {
                service_generation = service_generation.max(entry.generation);
                let Some(node) = crate::node_identity::key_from_name(&entry.node) else {
                    continue;
                };
                if entry.active
                    && entry.deployment_generation == deployment.generation
                    && deployment.replica_nodes.contains(&node)
                    && !ready_nodes.contains(&node)
                {
                    ready_nodes.push(node);
                }
            }
        }
        ready_nodes.sort_unstable();
        Some(IngressPlacement {
            deployment_generation: deployment.generation,
            service_generation,
            ready_nodes,
        })
    }

    pub fn release(&self, name: &[u8]) -> Option<ReleaseEntry> {
        self.releases.lock().get(name).cloned()
    }

    pub fn operational_binding(&self, profile_name: &[u8]) -> Option<OperationalBindingEntry> {
        self.operational_bindings.lock().get(profile_name).filter(|entry| entry.active).cloned()
    }

    pub fn operational_bindings(&self) -> Vec<(Vec<u8>, OperationalBindingEntry)> {
        self.operational_bindings
            .lock()
            .iter()
            .filter(|(_, entry)| entry.active)
            .map(|(name, entry)| (name.clone(), entry.clone()))
            .collect()
    }

    pub fn shutdown_intent(&self, node_key: u64) -> Option<ShutdownIntentEntry> {
        self.shutdown_intents.lock().get(&node_key).cloned()
    }

    pub fn ingress_policy(&self) -> Option<IngressPolicyEntry> {
        self.ingress_policy.lock().clone()
    }

    /// The latest committed capacity sample for `node_key`, or `None`.
    pub fn node_capacity(&self, node_key: u64) -> Option<NodeCapacityEntry> {
        self.node_capacity.lock().get(&node_key).copied()
    }

    /// Committed capacity samples in the shape the placement resolver takes.
    pub fn node_capacity_view(&self) -> crate::operations_admission::NodeCapacityView {
        self.node_capacity
            .lock()
            .iter()
            .map(|(node_key, entry)| {
                (
                    *node_key,
                    crate::operations_admission::NodeCapacity {
                        free_frames: entry.free_frames,
                        usable_frames: entry.usable_frames,
                        cpu_load_permille: entry.cpu_load_permille,
                    },
                )
            })
            .collect()
    }

    /// Nodes whose signed shutdown intent has committed, paired with the
    /// replicated intent generation. Ingress treats these members as
    /// draining: they remain trusted/routable for existing flows but stop
    /// receiving new ones before local service teardown begins.
    pub fn ingress_draining_nodes(&self) -> Vec<(u64, u64)> {
        self.shutdown_intents
            .lock()
            .iter()
            .map(|(node_key, entry)| (*node_key, entry.generation))
            .collect()
    }

    /// Whether `name` is registered to this node.
    pub fn is_local(&self, name: &[u8], local_node: &[u8]) -> bool {
        self.lookup_owner(name, local_node).is_some()
    }

    pub fn registered_count(&self) -> usize {
        self.entries.lock().values().filter(|entry| entry.active && !entry.node.is_empty()).count()
            + self
                .deployment_replicas
                .lock()
                .values()
                .flat_map(BTreeMap::values)
                .filter(|entry| entry.active && !entry.node.is_empty())
                .count()
    }

    pub fn deployment_count(&self) -> usize {
        self.deployments.lock().len()
    }

    /// The cluster's Ed25519 public key committed by the key ceremony, or
    /// `None` before the first ceremony.
    pub fn cluster_key(&self) -> Option<[u8; 32]> {
        *self.cluster_key.lock()
    }

    /// Snapshot copy of the whole `name -> {node, generation}` catalog.
    pub fn entries(&self) -> alloc::vec::Vec<(alloc::vec::Vec<u8>, CatalogEntry)> {
        let entries = self.entries.lock();
        let mut result = entries
            .iter()
            .filter(|(_, entry)| entry.active && !entry.node.is_empty())
            .map(|(name, entry)| (name.clone(), entry.clone()))
            .collect::<Vec<_>>();
        for (name, replicas) in self.deployment_replicas.lock().iter() {
            result.extend(
                replicas
                    .values()
                    .filter(|entry| entry.active && !entry.node.is_empty())
                    .map(|entry| (name.clone(), entry.clone())),
            );
        }
        result
    }

    fn apply_command(&self, command: &[u8]) -> Vec<u8> {
        match command.first().copied() {
            Some(CMD_REGISTER) => {
                let Some((name, after_name)) = take_len_bytes(command, 1) else {
                    return Vec::new();
                };
                let Some((node, after_node)) = take_len_bytes(command, after_name) else {
                    return Vec::new();
                };
                let deployment_generation = if command.len() == after_node {
                    0
                } else if command.len() == after_node + 8 {
                    u64::from_le_bytes(
                        command[after_node..after_node + 8].try_into().unwrap_or_default(),
                    )
                } else {
                    return Vec::new();
                };
                if deployment_generation != 0 {
                    let Some(node_key) = crate::node_identity::key_from_name(node) else {
                        return Vec::new();
                    };
                    let desired = self.deployments.lock().get(name).cloned();
                    if !desired.is_some_and(|deployment| {
                        deployment.generation == deployment_generation
                            && deployment.replica_nodes.contains(&node_key)
                    }) {
                        return 0u64.to_le_bytes().to_vec();
                    }
                    let ordinary_generation =
                        self.entries.lock().get(name).map_or(0, |entry| entry.generation);
                    let mut replicas = self.deployment_replicas.lock();
                    let by_node = replicas.entry(name.to_vec()).or_default();
                    let generation = by_node
                        .values()
                        .fold(ordinary_generation, |latest, entry| latest.max(entry.generation))
                        .checked_add(1)
                        .filter(|generation| *generation <= i64::MAX as u64);
                    let Some(generation) = generation else {
                        return 0u64.to_le_bytes().to_vec();
                    };
                    by_node.insert(
                        node.to_vec(),
                        CatalogEntry {
                            node: node.to_vec(),
                            generation,
                            active: false,
                            deployment_generation,
                        },
                    );
                    return generation.to_le_bytes().to_vec();
                }
                let mut entries = self.entries.lock();
                if !entries.contains_key(name) && entries.len() >= MAX_ENTRIES {
                    return 0u64.to_le_bytes().to_vec();
                }
                let generation = match entries.get(name) {
                    Some(entry) => entry
                        .generation
                        .checked_add(1)
                        .filter(|generation| *generation <= i64::MAX as u64),
                    None => Some(1),
                };
                let Some(generation) = generation else {
                    // Generation zero is the protocol's failed-prepare
                    // result. Never saturate and reuse the current generation:
                    // that would let a delayed activation or unregister
                    // mutate a logically newer service instance.
                    return 0u64.to_le_bytes().to_vec();
                };
                entries.insert(
                    name.to_vec(),
                    CatalogEntry {
                        node: node.to_vec(),
                        generation,
                        active: false,
                        deployment_generation,
                    },
                );
                generation.to_le_bytes().to_vec()
            }
            Some(CMD_UNREGISTER) => {
                let Some((name, _)) = take_len_bytes(command, 1) else {
                    return Vec::new();
                };
                if let Some(entry) = self.entries.lock().get_mut(name) {
                    entry.node.clear();
                    entry.active = false;
                }
                if let Some(entries) = self.deployment_replicas.lock().get_mut(name) {
                    for entry in entries.values_mut() {
                        entry.active = false;
                    }
                }
                Vec::new()
            }
            Some(CMD_ACTIVATE) => {
                let Some((name, after_name)) = take_len_bytes(command, 1) else {
                    return Vec::new();
                };
                let Some(bytes) = command.get(after_name..after_name.saturating_add(8)) else {
                    return Vec::new();
                };
                let Ok(bytes) = <[u8; 8]>::try_from(bytes) else {
                    return Vec::new();
                };
                let generation = u64::from_le_bytes(bytes);
                {
                    let mut entries = self.entries.lock();
                    if let Some(entry) = entries.get_mut(name)
                        && entry.generation == generation
                        && !entry.node.is_empty()
                    {
                        entry.active = true;
                        return generation.to_le_bytes().to_vec();
                    }
                }
                let mut replicas = self.deployment_replicas.lock();
                let Some(entry) = replicas.get_mut(name).and_then(|entries| {
                    entries.values_mut().find(|entry| entry.generation == generation)
                }) else {
                    return Vec::new();
                };
                if entry.node.is_empty() {
                    return Vec::new();
                }
                entry.active = true;
                generation.to_le_bytes().to_vec()
            }
            Some(CMD_UNREGISTER_GENERATION) => {
                let Some((name, after_name)) = take_len_bytes(command, 1) else {
                    return Vec::new();
                };
                let Some((node, after_node)) = take_len_bytes(command, after_name) else {
                    return Vec::new();
                };
                let Some(bytes) = command.get(after_node..after_node.saturating_add(8)) else {
                    return Vec::new();
                };
                let Ok(bytes) = <[u8; 8]>::try_from(bytes) else {
                    return Vec::new();
                };
                let generation = u64::from_le_bytes(bytes);
                {
                    let mut entries = self.entries.lock();
                    if let Some(entry) = entries.get_mut(name)
                        && entry.active
                        && entry.node == node
                        && entry.generation == generation
                    {
                        entry.node.clear();
                        entry.active = false;
                        return generation.to_le_bytes().to_vec();
                    }
                }
                let mut replicas = self.deployment_replicas.lock();
                let Some(entry) = replicas.get_mut(name).and_then(|entries| entries.get_mut(node))
                else {
                    return Vec::new();
                };
                if !entry.active || entry.generation != generation {
                    return Vec::new();
                }
                entry.active = false;
                generation.to_le_bytes().to_vec()
            }
            Some(CMD_DEPLOY) => {
                let Some((artifact, after_artifact)) = take_len_bytes(command, 1) else {
                    return Vec::new();
                };
                let (object_id, after_object) =
                    read_u64(command, after_artifact).unwrap_or((0, after_artifact));
                let (node_key, after_node) =
                    read_u64(command, after_object).unwrap_or((0, after_object));
                let Some(artifact_digest) = command.get(after_node..after_node.saturating_add(32))
                else {
                    return Vec::new();
                };
                let Ok(artifact_digest) = <[u8; 32]>::try_from(artifact_digest) else {
                    return Vec::new();
                };
                let after_digest = after_node + 32;
                let descriptor = if after_digest == command.len() {
                    Vec::new()
                } else {
                    let Some((descriptor, after_descriptor)) =
                        take_len_bytes(command, after_digest)
                    else {
                        return Vec::new();
                    };
                    if after_descriptor != command.len()
                        || descriptor.len() > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN
                    {
                        return Vec::new();
                    }
                    descriptor.to_vec()
                };
                let mut deployments = self.deployments.lock();
                if let Some(existing) = deployments.get(artifact)
                    && existing.object_id == object_id
                    && existing.node_key == node_key
                    && existing.artifact_digest == artifact_digest
                    && existing.descriptor == descriptor
                {
                    // Admission may be retried after the follower-to-leader
                    // reply is lost. Make exact desired state idempotent in
                    // the replicated state machine, not merely at ingress.
                    return existing.generation.to_le_bytes().to_vec();
                }
                let generation = match deployments.get(artifact) {
                    Some(entry) => entry.generation.checked_add(1),
                    None => Some(1),
                };
                let Some(generation) = generation else {
                    return 0u64.to_le_bytes().to_vec();
                };
                if !deployments.contains_key(artifact) && deployments.len() >= MAX_DEPLOYMENTS {
                    return crate::clusterctl::ERR_CAPACITY.to_le_bytes().to_vec();
                }
                deployments.insert(
                    artifact.to_vec(),
                    DeploymentEntry {
                        object_id,
                        node_key,
                        replica_nodes: (node_key != 0).then_some(node_key).into_iter().collect(),
                        generation,
                        artifact_digest,
                        descriptor,
                    },
                );
                drop(deployments);
                if let Some(replicas) = self.deployment_replicas.lock().get_mut(artifact) {
                    for replica in replicas.values_mut() {
                        replica.active = false;
                    }
                }
                generation.to_le_bytes().to_vec()
            }
            Some(CMD_REASSIGN) => {
                let Some((artifact, after_artifact)) = take_len_bytes(command, 1) else {
                    return Vec::new();
                };
                let Some((expected_generation, after_generation)) =
                    read_u64(command, after_artifact)
                else {
                    return Vec::new();
                };
                let Some((node_count, mut position)) = read_u16(command, after_generation) else {
                    return Vec::new();
                };
                if node_count == 0 {
                    return Vec::new();
                }
                let mut nodes = Vec::with_capacity(
                    usize::from(node_count).min(command.len().saturating_sub(position)),
                );
                let mut seen = BTreeSet::new();
                for _ in 0..node_count {
                    let Some((node, next)) = read_u64(command, position) else {
                        return Vec::new();
                    };
                    if node == 0 || !seen.insert(node) {
                        return Vec::new();
                    }
                    nodes.push(node);
                    position = next;
                }
                if position != command.len() {
                    return Vec::new();
                }
                nodes.sort_unstable();
                let mut deployments = self.deployments.lock();
                let Some(current) = deployments.get_mut(artifact) else {
                    return Vec::new();
                };
                if current.generation != expected_generation || current.descriptor.is_empty() {
                    return 0u64.to_le_bytes().to_vec();
                }
                let Some(descriptor) = charlotte_launch::deployment::decode(&current.descriptor)
                else {
                    return Vec::new();
                };
                let every_node = descriptor.placement.flags
                    & charlotte_launch::placement::EVERY_ELIGIBLE_NODE
                    != 0;
                if descriptor.node_key != 0
                    || (!every_node && nodes.len() != usize::from(descriptor.placement.replicas))
                {
                    return Vec::new();
                }
                if current.replica_nodes == nodes {
                    return current.generation.to_le_bytes().to_vec();
                }
                let Some(generation) = current
                    .generation
                    .checked_add(1)
                    .filter(|generation| *generation <= i64::MAX as u64)
                else {
                    return 0u64.to_le_bytes().to_vec();
                };
                current.node_key = nodes[0];
                current.replica_nodes = nodes;
                current.generation = generation;
                drop(deployments);
                if let Some(replicas) = self.deployment_replicas.lock().get_mut(artifact) {
                    for replica in replicas.values_mut() {
                        replica.active = false;
                    }
                }
                generation.to_le_bytes().to_vec()
            }
            Some(CMD_RELEASE) | Some(CMD_RELEASE_REPLICAS) => {
                let Some((envelope_bytes, after_envelope)) = take_len_bytes(command, 1) else {
                    return Vec::new();
                };
                let Some((descriptor_count, mut assignment_position)) =
                    read_u16(command, after_envelope)
                else {
                    return Vec::new();
                };
                let mut assignments = Vec::with_capacity(
                    usize::from(descriptor_count)
                        .min(command.len().saturating_sub(assignment_position)),
                );
                if command[0] == CMD_RELEASE {
                    for _ in 0..descriptor_count {
                        let Some((node, next)) = read_u64(command, assignment_position) else {
                            return Vec::new();
                        };
                        if node == 0 {
                            return Vec::new();
                        }
                        assignments.push(alloc::vec![node]);
                        assignment_position = next;
                    }
                } else {
                    for _ in 0..descriptor_count {
                        let Some((replica_count, after_count)) =
                            read_u16(command, assignment_position)
                        else {
                            return Vec::new();
                        };
                        assignment_position = after_count;
                        if replica_count == 0 {
                            return Vec::new();
                        }
                        let mut nodes = Vec::with_capacity(
                            usize::from(replica_count)
                                .min(command.len().saturating_sub(assignment_position)),
                        );
                        let mut seen = BTreeSet::new();
                        for _ in 0..replica_count {
                            let Some((node, next)) = read_u64(command, assignment_position) else {
                                return Vec::new();
                            };
                            if node == 0 || !seen.insert(node) {
                                return Vec::new();
                            }
                            nodes.push(node);
                            assignment_position = next;
                        }
                        nodes.sort_unstable();
                        assignments.push(nodes);
                    }
                }
                // The build-time key is the bootstrap trust anchor. A key
                // ceremony commits that same key for join/snapshot state,
                // but release admission must also work before the optional
                // ceremony on the first node.
                let cluster_key =
                    (*self.cluster_key.lock()).unwrap_or(self.bootstrap_deployment_key);
                if charlotte_launch::release::verify(envelope_bytes, &cluster_key)
                    != charlotte_launch::release::VerifyOutcome::Valid
                {
                    return crate::clusterctl::ERR_UNTRUSTED_DESCRIPTOR.to_le_bytes().to_vec();
                }
                let Some(envelope) = charlotte_launch::release::decode(envelope_bytes) else {
                    return crate::clusterctl::ERR_UNTRUSTED_DESCRIPTOR.to_le_bytes().to_vec();
                };
                if usize::from(descriptor_count) != envelope.descriptors().count() {
                    return Vec::new();
                }
                let Some(operational_tail) =
                    parse_operational_tail(command, assignment_position, &envelope)
                else {
                    return Vec::new();
                };
                let has_operational_tail = operational_tail.is_some();
                let release_digest = charlotte_launch::sha256::digest(envelope_bytes);

                let mut releases = self.releases.lock();
                let existing_release = releases.get(envelope.release_name);
                let mut release_exact = false;
                let release_generation = match existing_release {
                    Some(existing) => {
                        let Some(previous) = charlotte_launch::release::decode(&existing.envelope)
                        else {
                            return Vec::new();
                        };
                        if envelope.sequence < previous.sequence {
                            return crate::clusterctl::ERR_STALE_DESCRIPTOR.to_le_bytes().to_vec();
                        }
                        if envelope.sequence == previous.sequence {
                            if existing.envelope != envelope_bytes {
                                return crate::clusterctl::ERR_CONFLICTING_DESCRIPTOR
                                    .to_le_bytes()
                                    .to_vec();
                            }
                            release_exact = true;
                            Some(existing.generation)
                        } else {
                            existing.generation.checked_add(1)
                        }
                    }
                    None => Some(1),
                }
                .filter(|generation| *generation <= i64::MAX as u64);
                let Some(release_generation) = release_generation else {
                    return Vec::new();
                };
                if let Some(operations) = &operational_tail
                    && let Some(existing) = existing_release
                {
                    if operations.bundle_sequence < existing.operations_sequence {
                        return crate::clusterctl::ERR_STALE_DESCRIPTOR.to_le_bytes().to_vec();
                    }
                    if operations.bundle_sequence == existing.operations_sequence {
                        return if release_exact
                            && operations.bundle_digest == existing.operations_bundle_digest
                        {
                            (existing.generation as i64).to_le_bytes().to_vec()
                        } else {
                            crate::clusterctl::ERR_CONFLICTING_DESCRIPTOR.to_le_bytes().to_vec()
                        };
                    }
                } else if release_exact {
                    return (release_generation as i64).to_le_bytes().to_vec();
                }

                let mut deployments = self.deployments.lock();
                let mut planned_deployments = Vec::with_capacity(
                    usize::from(descriptor_count).min(command.len().saturating_sub(after_envelope)),
                );
                for (index, descriptor_bytes) in envelope.descriptors().enumerate() {
                    let Some(descriptor) = charlotte_launch::deployment::decode(descriptor_bytes)
                    else {
                        return Vec::new();
                    };
                    let assigned_nodes = &assignments[index];
                    let every_node = descriptor.placement.flags
                        & charlotte_launch::placement::EVERY_ELIGIBLE_NODE
                        != 0;
                    if (descriptor.node_key != 0
                        && assigned_nodes.as_slice() != [descriptor.node_key])
                        || (descriptor.node_key == 0
                            && !every_node
                            && assigned_nodes.len() != usize::from(descriptor.placement.replicas))
                    {
                        return Vec::new();
                    }
                    let (generation, effective_nodes) =
                        match deployments.get(descriptor.artifact_name) {
                            Some(current) if !current.descriptor.is_empty() => {
                                let Some(previous) =
                                    charlotte_launch::deployment::decode(&current.descriptor)
                                else {
                                    return Vec::new();
                                };
                                if descriptor.sequence < previous.sequence {
                                    return crate::clusterctl::ERR_STALE_DESCRIPTOR
                                        .to_le_bytes()
                                        .to_vec();
                                }
                                if descriptor.sequence == previous.sequence {
                                    if current.descriptor != descriptor_bytes {
                                        return crate::clusterctl::ERR_CONFLICTING_DESCRIPTOR
                                            .to_le_bytes()
                                            .to_vec();
                                    }
                                    (current.generation, current.replica_nodes.clone())
                                } else {
                                    let Some(generation) = current
                                        .generation
                                        .checked_add(1)
                                        .filter(|generation| *generation <= i64::MAX as u64)
                                    else {
                                        return Vec::new();
                                    };
                                    (generation, assigned_nodes.clone())
                                }
                            }
                            Some(current) => {
                                let Some(generation) = current
                                    .generation
                                    .checked_add(1)
                                    .filter(|generation| *generation <= i64::MAX as u64)
                                else {
                                    return Vec::new();
                                };
                                (generation, assigned_nodes.clone())
                            }
                            None => (1, assigned_nodes.clone()),
                        };
                    let effective_node = effective_nodes.first().copied().unwrap_or(0);
                    planned_deployments.push((
                        descriptor.artifact_name.to_vec(),
                        DeploymentEntry {
                            object_id: charlotte_launch::artifact_object_id(
                                descriptor.artifact_name,
                            ),
                            node_key: effective_node,
                            replica_nodes: effective_nodes,
                            generation,
                            artifact_digest: descriptor.artifact_digest,
                            descriptor: descriptor_bytes.to_vec(),
                        },
                    ));
                }

                let mut operational_bindings = self.operational_bindings.lock();
                let mut planned_operations = Vec::new();
                let (operations_sequence, operations_bundle_digest) = if let Some(operations) =
                    operational_tail
                {
                    planned_operations.reserve(operations.bindings.len());
                    for binding in operations.bindings {
                        let current = operational_bindings.get(binding.profile_name.as_slice());
                        if current.is_some_and(|entry| {
                            entry.active && entry.release_name != envelope.release_name
                        }) {
                            return crate::clusterctl::ERR_CONFLICTING_DESCRIPTOR
                                .to_le_bytes()
                                .to_vec();
                        }
                        let exact = current.is_some_and(|entry| {
                            entry.release_name == envelope.release_name
                                && entry.release_digest == release_digest
                                && entry.target_artifact == binding.target_artifact
                                && entry.object_key == binding.object_key
                                && entry.envelope_digest == binding.envelope_digest
                                && entry.profile_kind == binding.profile_kind
                                && entry.sequence == binding.sequence
                                && entry.expires_unix_seconds == binding.expires_unix_seconds
                                && entry.recipient_key_id == binding.recipient_key_id
                                && entry.signing_key_id == binding.signing_key_id
                                && entry.authorization_signature == binding.authorization_signature
                        });
                        if let Some(entry) = current {
                            if binding.sequence < entry.sequence {
                                return crate::clusterctl::ERR_STALE_DESCRIPTOR
                                    .to_le_bytes()
                                    .to_vec();
                            }
                            if binding.sequence == entry.sequence && !exact {
                                return crate::clusterctl::ERR_CONFLICTING_DESCRIPTOR
                                    .to_le_bytes()
                                    .to_vec();
                            }
                        }
                        let generation = if exact {
                            current.map(|entry| entry.generation)
                        } else {
                            current.map_or(Some(1), |entry| entry.generation.checked_add(1))
                        };
                        let Some(generation) =
                            generation.filter(|generation| *generation <= i64::MAX as u64)
                        else {
                            return Vec::new();
                        };
                        planned_operations.push((
                            binding.profile_name.clone(),
                            OperationalBindingEntry {
                                generation,
                                active: true,
                                release_name: envelope.release_name.to_vec(),
                                release_digest,
                                bundle_sequence: operations.bundle_sequence,
                                bundle_digest: operations.bundle_digest,
                                target_artifact: binding.target_artifact,
                                object_key: binding.object_key,
                                envelope_digest: binding.envelope_digest,
                                profile_kind: binding.profile_kind,
                                sequence: binding.sequence,
                                expires_unix_seconds: binding.expires_unix_seconds,
                                recipient_key_id: binding.recipient_key_id,
                                signing_key_id: binding.signing_key_id,
                                authorization_signature: binding.authorization_signature,
                            },
                        ));
                    }
                    (operations.bundle_sequence, operations.bundle_digest)
                } else {
                    existing_release.map_or((0, [0; 32]), |entry| {
                        (entry.operations_sequence, entry.operations_bundle_digest)
                    })
                };

                // A bundle is the complete operational set for its release.
                // Advancing the release without a bundle also retires bindings
                // tied to the old release digest. Keep inactive entries as
                // monotonic sequence tombstones.
                if !release_exact || has_operational_tail {
                    let retained_profiles = planned_operations
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect::<BTreeSet<_>>();
                    for (name, current) in operational_bindings.iter() {
                        if current.active
                            && current.release_name == envelope.release_name
                            && !retained_profiles.contains(name)
                        {
                            let Some(generation) = current.generation.checked_add(1) else {
                                return Vec::new();
                            };
                            let mut retired = current.clone();
                            retired.generation = generation;
                            retired.active = false;
                            planned_operations.push((name.clone(), retired));
                        }
                    }
                }

                // Bound each replicated collection. Replacements of existing
                // keys always succeed; only admitting new keys can exceed a
                // cap.
                let new_deployments = planned_deployments
                    .iter()
                    .filter(|(artifact, _)| !deployments.contains_key(artifact.as_slice()))
                    .count();
                if deployments.len() + new_deployments > MAX_DEPLOYMENTS {
                    return crate::clusterctl::ERR_CAPACITY.to_le_bytes().to_vec();
                }
                let new_bindings = planned_operations
                    .iter()
                    .filter(|(profile, _)| !operational_bindings.contains_key(profile.as_slice()))
                    .count();
                if operational_bindings.len() + new_bindings > MAX_OPERATIONAL_BINDINGS {
                    return crate::clusterctl::ERR_CAPACITY.to_le_bytes().to_vec();
                }
                if !releases.contains_key(envelope.release_name) && releases.len() >= MAX_RELEASES {
                    return crate::clusterctl::ERR_CAPACITY.to_le_bytes().to_vec();
                }

                let changed_artifacts = planned_deployments
                    .iter()
                    .filter(|(artifact, entry)| {
                        deployments
                            .get(artifact.as_slice())
                            .is_none_or(|current| current.generation != entry.generation)
                    })
                    .map(|(artifact, _)| artifact.clone())
                    .collect::<Vec<_>>();
                for (artifact, entry) in planned_deployments {
                    deployments.insert(artifact, entry);
                }
                drop(deployments);
                if !changed_artifacts.is_empty() {
                    let mut replicas = self.deployment_replicas.lock();
                    for artifact in changed_artifacts {
                        if let Some(entries) = replicas.get_mut(artifact.as_slice()) {
                            for entry in entries.values_mut() {
                                entry.active = false;
                            }
                        }
                    }
                }
                for (profile, entry) in planned_operations {
                    operational_bindings.insert(profile, entry);
                }
                releases.insert(
                    envelope.release_name.to_vec(),
                    ReleaseEntry {
                        generation: release_generation,
                        envelope: envelope_bytes.to_vec(),
                        operations_sequence,
                        operations_bundle_digest,
                    },
                );
                (release_generation as i64).to_le_bytes().to_vec()
            }
            Some(CMD_SET_CLUSTER_KEY) => {
                let Some(key) = command.get(1..1 + 32) else {
                    return Vec::new();
                };
                let Ok(key) = <[u8; 32]>::try_from(key) else {
                    return Vec::new();
                };
                let mut current = self.cluster_key.lock();
                let mut generation = self.cluster_key_generation.lock();
                if let Some(existing) = *current {
                    // Establishment is idempotent, not an unauthenticated key
                    // rotation exposure. Rotation needs a separately
                    // authorized protocol and overlap policy.
                    if existing != key {
                        return Vec::new();
                    }
                    return generation.to_le_bytes().to_vec();
                }
                *generation = 1;
                *current = Some(key);
                generation.to_le_bytes().to_vec()
            }
            Some(CMD_SHUTDOWN) => {
                let Some((envelope, after_envelope)) = take_len_bytes(command, 1) else {
                    return Vec::new();
                };
                if after_envelope != command.len() {
                    return Vec::new();
                }
                let cluster_key =
                    (*self.cluster_key.lock()).unwrap_or(self.bootstrap_deployment_key);
                if charlotte_launch::shutdown::verify(envelope, &cluster_key)
                    != charlotte_launch::shutdown::VerifyOutcome::Valid
                {
                    return crate::clusterctl::ERR_UNTRUSTED_DESCRIPTOR.to_le_bytes().to_vec();
                }
                let Some(fields) = charlotte_launch::shutdown::decode(envelope) else {
                    return Vec::new();
                };
                let mut intents = self.shutdown_intents.lock();
                let generation = match intents.get(&fields.target_node) {
                    Some(existing) => {
                        let Some(previous) = charlotte_launch::shutdown::decode(&existing.envelope)
                        else {
                            return Vec::new();
                        };
                        if fields.sequence < previous.sequence {
                            return crate::clusterctl::ERR_STALE_DESCRIPTOR.to_le_bytes().to_vec();
                        }
                        if fields.sequence == previous.sequence {
                            return if existing.envelope == envelope {
                                (existing.generation as i64).to_le_bytes().to_vec()
                            } else {
                                crate::clusterctl::ERR_CONFLICTING_DESCRIPTOR.to_le_bytes().to_vec()
                            };
                        }
                        existing.generation.checked_add(1)
                    }
                    None => Some(1),
                }
                .filter(|generation| *generation <= i64::MAX as u64);
                let Some(generation) = generation else {
                    return Vec::new();
                };
                if !intents.contains_key(&fields.target_node)
                    && intents.len() >= MAX_SHUTDOWN_INTENTS
                {
                    return crate::clusterctl::ERR_CAPACITY.to_le_bytes().to_vec();
                }
                intents.insert(
                    fields.target_node,
                    ShutdownIntentEntry {
                        generation,
                        envelope: envelope.to_vec(),
                    },
                );
                (generation as i64).to_le_bytes().to_vec()
            }
            Some(CMD_INGRESS_POLICY) => {
                let Some((envelope, after_envelope)) = take_len_bytes(command, 1) else {
                    return Vec::new();
                };
                if after_envelope != command.len()
                    || charlotte_launch::ingress_policy::verify(
                        envelope,
                        &self.cluster_id,
                        &self.bootstrap_operations_key,
                    ) != charlotte_launch::ingress_policy::VerifyOutcome::Valid
                {
                    return crate::clusterctl::ERR_UNTRUSTED_DESCRIPTOR.to_le_bytes().to_vec();
                }
                let Some(policy) = charlotte_launch::ingress_policy::decode(envelope) else {
                    return Vec::new();
                };
                let mut current = self.ingress_policy.lock();
                let generation = match current.as_ref() {
                    Some(existing) => {
                        let Some(previous) =
                            charlotte_launch::ingress_policy::decode(&existing.envelope)
                        else {
                            return Vec::new();
                        };
                        if policy.sequence < previous.sequence {
                            return crate::clusterctl::ERR_STALE_DESCRIPTOR.to_le_bytes().to_vec();
                        }
                        if policy.sequence == previous.sequence {
                            return if existing.envelope == envelope {
                                (existing.generation as i64).to_le_bytes().to_vec()
                            } else {
                                crate::clusterctl::ERR_CONFLICTING_DESCRIPTOR.to_le_bytes().to_vec()
                            };
                        }
                        existing.generation.checked_add(1)
                    }
                    None => Some(1),
                }
                .filter(|generation| *generation <= i64::MAX as u64);
                let Some(generation) = generation else {
                    return Vec::new();
                };
                *current = Some(IngressPolicyEntry {
                    generation,
                    envelope: envelope.to_vec(),
                });
                (generation as i64).to_le_bytes().to_vec()
            }
            Some(CMD_NODE_CAPACITY) => {
                let Some(entry) = decode_node_capacity(command) else {
                    return Vec::new();
                };
                let mut samples = self.node_capacity.lock();
                if let Some(existing) = samples.get(&entry.node_key) {
                    // Within one boot, samples are monotonic by epoch. A new
                    // boot nonce supersedes the previous boot's samples so a
                    // restart with a reset clock is not fenced out.
                    if existing.boot_nonce == entry.boot_nonce && entry.epoch <= existing.epoch {
                        return Vec::new();
                    }
                } else if samples.len() >= MAX_NODE_CAPACITY_ENTRIES {
                    return crate::clusterctl::ERR_CAPACITY.to_le_bytes().to_vec();
                }
                samples.insert(entry.node_key, entry);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

impl StateMachine for NameCatalog {
    fn apply(&self, _term: u64, command: &[u8]) {
        let result = self.apply_command(command);
        *self.last_apply.lock() = if result.is_empty() {
            None
        } else {
            Some(result)
        };
    }

    fn apply_with_result(&self, _term: u64, command: &[u8]) -> Vec<u8> {
        let result = self.apply_command(command);
        *self.last_apply.lock() = if result.is_empty() {
            None
        } else {
            Some(result.clone())
        };
        result
    }

    fn snapshot(&self) -> Vec<u8> {
        self.snapshot_bytes()
    }

    fn restore(&self, snapshot_data: &[u8]) {
        self.restore_bytes(snapshot_data);
    }

    fn reset(&self) {
        self.entries.lock().clear();
        self.deployment_replicas.lock().clear();
        self.deployments.lock().clear();
        self.releases.lock().clear();
        self.operational_bindings.lock().clear();
        self.shutdown_intents.lock().clear();
        self.node_capacity.lock().clear();
        *self.ingress_policy.lock() = None;
        *self.cluster_key.lock() = None;
        *self.cluster_key_generation.lock() = 0;
        *self.last_apply.lock() = None;
    }

    fn as_queryable(&self) -> Option<&dyn QueryableStateMachine> {
        Some(self)
    }
}

impl QueryableStateMachine for NameCatalog {
    fn query(&self, query: &[u8]) -> Vec<u8> {
        match query.first().copied() {
            Some(QUERY_LOOKUP) => {
                let name = query.get(1..).unwrap_or_default();
                self.lookup(name).map_or_else(Vec::new, |entry| {
                    let mut result = Vec::with_capacity(8 + entry.node.len());
                    result.extend_from_slice(&entry.generation.to_le_bytes());
                    result.extend_from_slice(&entry.node);
                    result
                })
            }
            Some(QUERY_DEPLOY) => {
                let artifact = query.get(1..).unwrap_or_default();
                self.deployment(artifact)
                    .and_then(|entry| encode_deployment_result(&entry))
                    .unwrap_or_default()
            }
            Some(QUERY_SHUTDOWN) => {
                let node_key = query
                    .get(1..9)
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                    .map(u64::from_le_bytes);
                node_key.and_then(|node_key| self.shutdown_intent(node_key)).map_or_else(
                    Vec::new,
                    |entry| {
                        let mut result = Vec::with_capacity(8 + entry.envelope.len());
                        result.extend_from_slice(&entry.generation.to_le_bytes());
                        result.extend_from_slice(&entry.envelope);
                        result
                    },
                )
            }
            Some(QUERY_INGRESS_POLICY) => self.ingress_policy().map_or_else(Vec::new, |entry| {
                let mut result = Vec::with_capacity(8 + entry.envelope.len());
                result.extend_from_slice(&entry.generation.to_le_bytes());
                result.extend_from_slice(&entry.envelope);
                result
            }),
            _ => Vec::new(),
        }
    }
}

fn take_len_bytes(bytes: &[u8], start: usize) -> Option<(&[u8], usize)> {
    if bytes.len() < start + 4 {
        return None;
    }
    let len = u32::from_le_bytes(bytes[start..start + 4].try_into().ok()?) as usize;
    let begin = start + 4;
    let end = begin.checked_add(len)?;
    if end > bytes.len() {
        return None;
    }
    Some((&bytes[begin..end], end))
}

fn read_u64(bytes: &[u8], start: usize) -> Option<(u64, usize)> {
    let end = start.checked_add(8)?;
    let value = u64::from_le_bytes(bytes.get(start..end)?.try_into().ok()?);
    Some((value, end))
}

fn read_u16(bytes: &[u8], start: usize) -> Option<(u16, usize)> {
    let end = start.checked_add(2)?;
    let value = u16::from_le_bytes(bytes.get(start..end)?.try_into().ok()?);
    Some((value, end))
}

fn decode_node_capacity(command: &[u8]) -> Option<NodeCapacityEntry> {
    if command.len() != 43 {
        return None;
    }
    let (node_key, position) = read_u64(command, 1)?;
    let (boot_nonce, position) = read_u64(command, position)?;
    let (epoch, position) = read_u64(command, position)?;
    let (free_frames, position) = read_u64(command, position)?;
    let (usable_frames, position) = read_u64(command, position)?;
    let (cpu_load_permille, end) = read_u16(command, position)?;
    if end != command.len()
        || node_key == 0
        || boot_nonce == 0
        || epoch == 0
        || usable_frames == 0
        || free_frames > usable_frames
        || cpu_load_permille > 1000
    {
        return None;
    }
    Some(NodeCapacityEntry {
        node_key,
        boot_nonce,
        epoch,
        free_frames,
        usable_frames,
        cpu_load_permille,
    })
}

/// Encode a register command: `{name, node}`.
pub fn encode_register(name: &[u8], node: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 4 + name.len() + 4 + node.len());
    buf.push(CMD_REGISTER);
    buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
    buf.extend_from_slice(name);
    buf.extend_from_slice(&(node.len() as u32).to_le_bytes());
    buf.extend_from_slice(node);
    buf
}

/// Encode a deployed application's registration, binding the active endpoint
/// to the exact desired deployment generation that launched it.
pub fn encode_register_deployment(name: &[u8], node: &[u8], deployment_generation: u64) -> Vec<u8> {
    let mut buf = encode_register(name, node);
    buf.extend_from_slice(&deployment_generation.to_le_bytes());
    buf
}

/// Encode an unregister command: `{name}`.
pub fn encode_unregister(name: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 4 + name.len());
    buf.push(CMD_UNREGISTER);
    buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
    buf.extend_from_slice(name);
    buf
}

/// Encode a generation- and owner-fenced unregister command. A delayed
/// command cannot tombstone a replacement generation or another node's
/// service with the same name.
pub fn encode_unregister_generation(name: &[u8], node: &[u8], generation: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 4 + name.len() + 4 + node.len() + 8);
    buf.push(CMD_UNREGISTER_GENERATION);
    buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
    buf.extend_from_slice(name);
    buf.extend_from_slice(&(node.len() as u32).to_le_bytes());
    buf.extend_from_slice(node);
    buf.extend_from_slice(&generation.to_le_bytes());
    buf
}

/// Activate the exact prepared generation after its node-local endpoint has
/// been published successfully.
pub fn encode_activate(name: &[u8], generation: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 4 + name.len() + 8);
    buf.push(CMD_ACTIVATE);
    buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
    buf.extend_from_slice(name);
    buf.extend_from_slice(&generation.to_le_bytes());
    buf
}

/// Decode the state-machine query result emitted by [`QueryableStateMachine`].
pub fn decode_query_result(bytes: &[u8]) -> Option<CatalogEntry> {
    if bytes.len() < 8 {
        return None;
    }
    let generation = u64::from_le_bytes(bytes[..8].try_into().ok()?);
    let node = bytes[8..].to_vec();
    if generation == 0 || node.is_empty() {
        return None;
    }
    Some(CatalogEntry {
        node,
        generation,
        active: true,
        deployment_generation: 0,
    })
}

/// Tagged query encoding for a name lookup.
pub fn encode_lookup_query(name: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + name.len());
    buf.push(QUERY_LOOKUP);
    buf.extend_from_slice(name);
    buf
}

/// Tagged query encoding for a deployment query.
pub fn encode_deploy_query(artifact: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + artifact.len());
    buf.push(QUERY_DEPLOY);
    buf.extend_from_slice(artifact);
    buf
}

/// Query the latest committed shutdown intent for `node_key`.
pub fn encode_shutdown_query(node_key: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(QUERY_SHUTDOWN);
    buf.extend_from_slice(&node_key.to_le_bytes());
    buf
}

pub fn decode_shutdown_result(bytes: &[u8]) -> Option<ShutdownIntentEntry> {
    if bytes.len() != 8 + charlotte_launch::shutdown::ENCODED_LEN {
        return None;
    }
    let generation = u64::from_le_bytes(bytes[..8].try_into().ok()?);
    let envelope = bytes[8..].to_vec();
    (generation != 0 && charlotte_launch::shutdown::decode(&envelope).is_some()).then_some(
        ShutdownIntentEntry {
            generation,
            envelope,
        },
    )
}

pub fn encode_ingress_policy_query() -> Vec<u8> {
    alloc::vec![QUERY_INGRESS_POLICY]
}

pub fn decode_ingress_policy_result(bytes: &[u8]) -> Option<IngressPolicyEntry> {
    if bytes.len() < 8 + charlotte_launch::ingress_policy::HEADER_LEN {
        return None;
    }
    let generation = u64::from_le_bytes(bytes[..8].try_into().ok()?);
    let envelope = bytes[8..].to_vec();
    (generation != 0 && charlotte_launch::ingress_policy::decode(&envelope).is_some()).then_some(
        IngressPolicyEntry {
            generation,
            envelope,
        },
    )
}

/// Decode the deployment record returned by a deployment query.
pub fn decode_deployment_result(bytes: &[u8]) -> Option<DeploymentEntry> {
    if bytes.len() < 56 {
        return None;
    }
    let (descriptor, replica_nodes) = if bytes.len() == 56 {
        (Vec::new(), Vec::new())
    } else {
        let descriptor_len =
            usize::try_from(u32::from_le_bytes(bytes.get(56..60)?.try_into().ok()?)).ok()?;
        if descriptor_len > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN {
            return None;
        }
        let descriptor_end = 60usize.checked_add(descriptor_len)?;
        let descriptor = bytes.get(60..descriptor_end)?.to_vec();
        let nodes = if descriptor_end == bytes.len() {
            Vec::new()
        } else {
            let count = usize::from(read_u16(bytes, descriptor_end)?.0);
            let nodes_end = descriptor_end.checked_add(2)?.checked_add(count.checked_mul(8)?)?;
            if nodes_end != bytes.len() {
                return None;
            }
            let mut nodes = Vec::with_capacity(count);
            let mut seen = BTreeSet::new();
            let mut position = descriptor_end + 2;
            for _ in 0..count {
                let (node, next) = read_u64(bytes, position)?;
                if node == 0 || !seen.insert(node) {
                    return None;
                }
                nodes.push(node);
                position = next;
            }
            nodes
        };
        (descriptor, nodes)
    };
    let node_key = u64::from_le_bytes(bytes[16..24].try_into().ok()?);
    let replica_nodes = if replica_nodes.is_empty() && node_key != 0 {
        alloc::vec![node_key]
    } else {
        replica_nodes
    };
    Some(DeploymentEntry {
        generation: u64::from_le_bytes(bytes[0..8].try_into().ok()?),
        object_id: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
        node_key,
        replica_nodes,
        artifact_digest: bytes[24..56].try_into().ok()?,
        descriptor,
    })
}

/// Encode a deployment query result, including the committed concrete
/// replica set after the backwards-compatible singleton prefix.
pub fn encode_deployment_result(entry: &DeploymentEntry) -> Option<Vec<u8>> {
    let count = u16::try_from(entry.replica_nodes.len()).ok()?;
    let mut bytes = Vec::with_capacity(62 + entry.descriptor.len() + entry.replica_nodes.len() * 8);
    bytes.extend_from_slice(&entry.generation.to_le_bytes());
    bytes.extend_from_slice(&entry.object_id.to_le_bytes());
    bytes.extend_from_slice(&entry.node_key.to_le_bytes());
    bytes.extend_from_slice(&entry.artifact_digest);
    bytes.extend_from_slice(&(entry.descriptor.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&entry.descriptor);
    bytes.extend_from_slice(&count.to_le_bytes());
    for node in &entry.replica_nodes {
        bytes.extend_from_slice(&node.to_le_bytes());
    }
    Some(bytes)
}

/// Encode a deployment command: assign `artifact` (stored at `object_id`) to
/// the node identified by `node_key`.
pub fn encode_deploy(
    artifact: &[u8],
    object_id: u64,
    node_key: u64,
    artifact_digest: &[u8; 32],
    descriptor: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 4 + artifact.len() + 16 + 32 + 4 + descriptor.len());
    buf.push(CMD_DEPLOY);
    buf.extend_from_slice(&(artifact.len() as u32).to_le_bytes());
    buf.extend_from_slice(artifact);
    buf.extend_from_slice(&object_id.to_le_bytes());
    buf.extend_from_slice(&node_key.to_le_bytes());
    buf.extend_from_slice(artifact_digest);
    buf.extend_from_slice(&(descriptor.len() as u32).to_le_bytes());
    buf.extend_from_slice(descriptor);
    buf
}

/// Commit a controller-computed replacement replica set while fencing the
/// deployment generation observed during planning.
pub fn encode_reassign(
    artifact: &[u8],
    expected_generation: u64,
    replica_nodes: &[u64],
) -> Option<Vec<u8>> {
    if expected_generation == 0
        || replica_nodes.is_empty()
        || replica_nodes.len() > u16::MAX as usize
        || replica_nodes.contains(&0)
        || replica_nodes
            .iter()
            .enumerate()
            .any(|(index, node)| replica_nodes[..index].contains(node))
    {
        return None;
    }
    let mut buf = Vec::with_capacity(1 + 4 + artifact.len() + 8 + 2 + replica_nodes.len() * 8);
    buf.push(CMD_REASSIGN);
    buf.extend_from_slice(&(artifact.len() as u32).to_le_bytes());
    buf.extend_from_slice(artifact);
    buf.extend_from_slice(&expected_generation.to_le_bytes());
    buf.extend_from_slice(&(replica_nodes.len() as u16).to_le_bytes());
    for node in replica_nodes {
        buf.extend_from_slice(&node.to_le_bytes());
    }
    (buf.len() <= catten_graft::types::MAX_COMMAND_BYTES).then_some(buf)
}

/// Encode one signed release and the leader-resolved node assignment for
/// every nested descriptor. The state machine verifies the envelope again
/// and applies the complete component set under one deployment-map lock.
pub fn encode_release(envelope: &[u8], assigned_nodes: &[u64]) -> Option<Vec<u8>> {
    let release = charlotte_launch::release::decode(envelope)?;
    if release.descriptors().count() != assigned_nodes.len()
        || assigned_nodes.len() > u16::MAX as usize
    {
        return None;
    }
    let mut buf = Vec::with_capacity(1 + 4 + envelope.len() + 2 + assigned_nodes.len() * 8);
    buf.push(CMD_RELEASE);
    buf.extend_from_slice(&(envelope.len() as u32).to_le_bytes());
    buf.extend_from_slice(envelope);
    buf.extend_from_slice(&(assigned_nodes.len() as u16).to_le_bytes());
    for node in assigned_nodes {
        buf.extend_from_slice(&node.to_le_bytes());
    }
    (buf.len() <= catten_graft::types::MAX_COMMAND_BYTES).then_some(buf)
}

/// Encode a signed release plus one concrete, non-empty, unique node set per
/// descriptor. The leader resolves policy to these assignments before Raft
/// submission; followers validate the cardinality against the signed policy.
pub fn encode_release_replicas(envelope: &[u8], assignments: &[Vec<u64>]) -> Option<Vec<u8>> {
    let release = charlotte_launch::release::decode(envelope)?;
    if release.descriptors().count() != assignments.len() || assignments.len() > u16::MAX as usize {
        return None;
    }
    let assignments_len = assignments.iter().try_fold(0usize, |total, nodes| {
        if nodes.is_empty()
            || nodes.len() > u16::MAX as usize
            || nodes.contains(&0)
            || nodes.iter().enumerate().any(|(index, node)| nodes[..index].contains(node))
        {
            None
        } else {
            total.checked_add(2 + nodes.len() * 8)
        }
    })?;
    let mut buf = Vec::with_capacity(1 + 4 + envelope.len() + 2 + assignments_len);
    buf.push(CMD_RELEASE_REPLICAS);
    buf.extend_from_slice(&(envelope.len() as u32).to_le_bytes());
    buf.extend_from_slice(envelope);
    buf.extend_from_slice(&(assignments.len() as u16).to_le_bytes());
    for nodes in assignments {
        buf.extend_from_slice(&(nodes.len() as u16).to_le_bytes());
        for node in nodes {
            buf.extend_from_slice(&node.to_le_bytes());
        }
    }
    (buf.len() <= catten_graft::types::MAX_COMMAND_BYTES).then_some(buf)
}

/// Compact an already verified `COPSBND2` admission bundle for the Raft log.
///
/// The large encrypted envelopes are transport proofs and remain in the
/// central object store. This command retains their signed identities,
/// routing metadata and replay fences alongside the exact release. The DNS
/// leader must call [`charlotte_launch::operations_bundle::verify`] before
/// constructing this trusted compact command.
pub fn encode_release_with_operations(
    bundle_bytes: &[u8],
    assigned_nodes: &[u64],
) -> Option<Vec<u8>> {
    let bundle = charlotte_launch::operations_bundle::decode(bundle_bytes)?;
    let mut command = encode_release(bundle.release, assigned_nodes)?;
    append_operational_tail(&mut command, bundle_bytes, &bundle)?;
    (command.len() <= catten_graft::types::MAX_COMMAND_BYTES).then_some(command)
}

fn append_operational_tail(
    command: &mut Vec<u8>,
    bundle_bytes: &[u8],
    bundle: &charlotte_launch::operations_bundle::Bundle<'_>,
) -> Option<()> {
    command.extend_from_slice(&bundle.sequence.to_le_bytes());
    command.extend_from_slice(&charlotte_launch::sha256::digest(bundle_bytes));
    let binding_count = u16::try_from(bundle.bindings().count()).ok()?;
    command.extend_from_slice(&binding_count.to_le_bytes());
    for binding in bundle.bindings() {
        let envelope = charlotte_launch::operations::decode(binding.envelope)?;
        command.extend_from_slice(&envelope.sequence.to_le_bytes());
        command.extend_from_slice(&envelope.expires_unix_seconds.to_le_bytes());
        command.extend_from_slice(&envelope.profile_kind.to_le_bytes());
        command
            .extend_from_slice(&u16::try_from(binding.target_artifact.len()).ok()?.to_le_bytes());
        command.extend_from_slice(&u16::try_from(envelope.profile_name.len()).ok()?.to_le_bytes());
        command.extend_from_slice(&u16::try_from(binding.object_key.len()).ok()?.to_le_bytes());
        command.extend_from_slice(&binding.envelope_digest);
        command.extend_from_slice(&envelope.recipient_key_id);
        command.extend_from_slice(&envelope.signing_key_id);
        command.extend_from_slice(&binding.authorization_signature);
        command.extend_from_slice(binding.target_artifact);
        command.extend_from_slice(envelope.profile_name);
        command.extend_from_slice(binding.object_key);
    }
    Some(())
}

/// Replica-set form of [`encode_release_with_operations`].
pub fn encode_release_replicas_with_operations(
    bundle_bytes: &[u8],
    assignments: &[Vec<u64>],
) -> Option<Vec<u8>> {
    let bundle = charlotte_launch::operations_bundle::decode(bundle_bytes)?;
    let mut command = encode_release_replicas(bundle.release, assignments)?;
    append_operational_tail(&mut command, bundle_bytes, &bundle)?;
    (command.len() <= catten_graft::types::MAX_COMMAND_BYTES).then_some(command)
}

/// Encode a key-ceremony command: commit the cluster's Ed25519 public key.
pub fn encode_set_cluster_key(key: &[u8; 32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 32);
    buf.push(CMD_SET_CLUSTER_KEY);
    buf.extend_from_slice(key);
    buf
}

/// Encode an operator-signed shutdown intent for deterministic Raft
/// admission. Signature and replay checks are repeated by the state machine.
pub fn encode_shutdown(envelope: &[u8]) -> Option<Vec<u8>> {
    charlotte_launch::shutdown::decode(envelope)?;
    let mut buf = Vec::with_capacity(1 + 4 + envelope.len());
    buf.push(CMD_SHUTDOWN);
    buf.extend_from_slice(&(envelope.len() as u32).to_le_bytes());
    buf.extend_from_slice(envelope);
    Some(buf)
}

/// Encode a complete operator-signed ingress-policy replacement. Signature,
/// cluster binding and replay checks are repeated by the state machine.
pub fn encode_ingress_policy(envelope: &[u8]) -> Option<Vec<u8>> {
    charlotte_launch::ingress_policy::decode(envelope)?;
    let mut buf = Vec::with_capacity(1 + 4 + envelope.len());
    buf.push(CMD_INGRESS_POLICY);
    buf.extend_from_slice(&(envelope.len() as u32).to_le_bytes());
    buf.extend_from_slice(envelope);
    Some(buf)
}

/// Encode a committed node-capacity sample. Range and replay checks are
/// repeated by the state machine.
pub fn encode_node_capacity(entry: &NodeCapacityEntry) -> Vec<u8> {
    let mut buf = Vec::with_capacity(43);
    buf.push(CMD_NODE_CAPACITY);
    buf.extend_from_slice(&entry.node_key.to_le_bytes());
    buf.extend_from_slice(&entry.boot_nonce.to_le_bytes());
    buf.extend_from_slice(&entry.epoch.to_le_bytes());
    buf.extend_from_slice(&entry.free_frames.to_le_bytes());
    buf.extend_from_slice(&entry.usable_frames.to_le_bytes());
    buf.extend_from_slice(&entry.cpu_load_permille.to_le_bytes());
    buf
}

/// The replicated catalog viewed as an immediate [`Catalog`]: answers come
/// from the *applied* state, so a resolved name is guaranteed to have
/// committed. Used by the event broker's lookups.
impl crate::broker::Catalog for NameCatalog {
    fn resolve(&self, name: &[u8]) -> Option<crate::broker::CatalogTarget> {
        self.lookup(name).map(|entry| crate::broker::CatalogTarget {
            generation: entry.generation,
            connection: 0,
        })
    }
}
