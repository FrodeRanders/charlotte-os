//! Replicated name catalog: the Raft state machine behind the distributed
//! name service.
//!
//! Maps `name -> {node_id, generation}`. Connections are
//! node-local capabilities and cannot be replicated, so only the *location* of
//! a registration is committed; resolving it to a connection stays a local
//! operation on the hosting node.
//!
//! The same state machine also carries the cluster **deployment manifest**
//! (`artifact -> {object_id, artifact_sha256, node_key, descriptor, generation}`) and the cluster's
//! **Ed25519 public key** (committed by the key ceremony). A deployment is a
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
//! ```
use alloc::{
    collections::BTreeMap,
    sync::Arc,
    vec::Vec,
};

use catten_graft::state_machine::{
    QueryableStateMachine,
    StateMachine,
};

const CMD_REGISTER: u8 = 0x01;
const CMD_UNREGISTER: u8 = 0x02;
const CMD_ACTIVATE: u8 = 0x03;
const CMD_UNREGISTER_GENERATION: u8 = 0x04;
const CMD_DEPLOY: u8 = 0x05;
const CMD_SET_CLUSTER_KEY: u8 = 0x07;
const CATALOG_MAGIC_V1: u64 = 0x4341_5441_4c4f_474d; // "CATALOGM"
const CATALOG_MAGIC_V2: u64 = 0x4341_5441_4c4f_4732; // "CATALOG2"
const CATALOG_MAGIC_V3: u64 = 0x4341_5441_4c4f_4733; // "CATALOG3"
const CATALOG_MAGIC_V4: u64 = 0x4341_5441_4c4f_4734; // "CATALOG4"
const CATALOG_MAGIC_V5: u64 = 0x4341_5441_4c4f_4735; // "CATALOG5"
const CATALOG_MAGIC_V6: u64 = 0x4341_5441_4c4f_4736; // "CATALOG6"
const CATALOG_MAGIC_V7: u64 = 0x4341_5441_4c4f_4737; // "CATALOG7"

/// Query tag prefix for a name lookup.
const QUERY_LOOKUP: u8 = 0x01;
/// Query tag prefix for a deployment query.
const QUERY_DEPLOY: u8 = 0x02;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    pub node: Vec<u8>,
    pub generation: u64,
    pub active: bool,
}

/// A replicated deployment record: the cluster's answer to "which node runs
/// this artifact, and from which object-store object?".
///
/// `node_key` is the packed cluster node identity (the FNV-1a of the node's
/// NIC MAC). The record needs no signature: the Raft consensus that
/// committed it is its authenticity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentEntry {
    pub object_id: u64,
    pub node_key: u64,
    pub generation: u64,
    /// Immutable content identity selected by this deployment generation.
    pub artifact_digest: [u8; 32],
    /// Signed, bounded deployment decision. Empty only for a legacy record.
    pub descriptor: Vec<u8>,
}

pub struct NameCatalog {
    entries: spin::Mutex<BTreeMap<Vec<u8>, CatalogEntry>>,
    deployments: spin::Mutex<BTreeMap<Vec<u8>, DeploymentEntry>>,
    cluster_key: spin::Mutex<Option<[u8; 32]>>,
    cluster_key_generation: spin::Mutex<u64>,
    last_apply: spin::Mutex<Option<Vec<u8>>>,
}

impl NameCatalog {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: spin::Mutex::new(BTreeMap::new()),
            deployments: spin::Mutex::new(BTreeMap::new()),
            cluster_key: spin::Mutex::new(None),
            cluster_key_generation: spin::Mutex::new(0),
            last_apply: spin::Mutex::new(None),
        })
    }

    /// The replicated owner and service generation for `name`, or `None`.
    pub fn lookup(&self, name: &[u8]) -> Option<CatalogEntry> {
        self.entries
            .lock()
            .get(name)
            .filter(|entry| entry.active && !entry.node.is_empty())
            .cloned()
    }

    /// The replicated deployment record for `artifact`, or `None`.
    pub fn deployment(&self, artifact: &[u8]) -> Option<DeploymentEntry> {
        self.deployments.lock().get(artifact).cloned()
    }

    /// Whether `name` is registered to this node.
    pub fn is_local(&self, name: &[u8], local_node: &[u8]) -> bool {
        self.lookup(name).is_some_and(|entry| entry.node == local_node)
    }

    pub fn registered_count(&self) -> usize {
        self.entries.lock().values().filter(|entry| entry.active && !entry.node.is_empty()).count()
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
        entries
            .iter()
            .filter(|(_, entry)| entry.active && !entry.node.is_empty())
            .map(|(name, entry)| (name.clone(), entry.clone()))
            .collect()
    }

    fn apply_command(&self, command: &[u8]) -> Vec<u8> {
        match command.first().copied() {
            Some(CMD_REGISTER) => {
                let Some((name, after_name)) = take_len_bytes(command, 1) else {
                    return Vec::new();
                };
                let Some((node, _)) = take_len_bytes(command, after_name) else {
                    return Vec::new();
                };
                let mut entries = self.entries.lock();
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
                let mut entries = self.entries.lock();
                let Some(entry) = entries.get_mut(name) else {
                    return Vec::new();
                };
                if entry.generation != generation || entry.node.is_empty() {
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
                let mut entries = self.entries.lock();
                let Some(entry) = entries.get_mut(name) else {
                    return Vec::new();
                };
                if !entry.active || entry.node != node || entry.generation != generation {
                    return Vec::new();
                }
                entry.node.clear();
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
                let generation = match deployments.get(artifact) {
                    Some(entry) => entry.generation.checked_add(1),
                    None => Some(1),
                };
                let Some(generation) = generation else {
                    return 0u64.to_le_bytes().to_vec();
                };
                deployments.insert(
                    artifact.to_vec(),
                    DeploymentEntry {
                        object_id,
                        node_key,
                        generation,
                        artifact_digest,
                        descriptor,
                    },
                );
                generation.to_le_bytes().to_vec()
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
            _ => Vec::new(),
        }
    }

    fn snapshot_bytes(&self) -> Vec<u8> {
        let entries = self.entries.lock();
        let deployments = self.deployments.lock();
        let mut size = 8 + 4; // magic + entry count
        for (name, entry) in entries.iter() {
            size += 4 + name.len() + 4 + entry.node.len() + 8 + 1;
        }
        // V7 appends signed deployment descriptors to the manifest records.
        // cluster key (present flag + 32 bytes).
        size += 4;
        for (artifact, entry) in deployments.iter() {
            size += 4 + artifact.len() + 8 + 8 + 8 + 32 + 4 + entry.descriptor.len();
        }
        size += 1 + 8 + 32;
        let mut buf = Vec::with_capacity(size);
        buf.extend_from_slice(&CATALOG_MAGIC_V7.to_le_bytes());
        buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (name, entry) in entries.iter() {
            buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
            buf.extend_from_slice(name);
            buf.extend_from_slice(&(entry.node.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.node);
            buf.extend_from_slice(&entry.generation.to_le_bytes());
            buf.push(u8::from(entry.active));
        }
        buf.extend_from_slice(&(deployments.len() as u32).to_le_bytes());
        for (artifact, entry) in deployments.iter() {
            buf.extend_from_slice(&(artifact.len() as u32).to_le_bytes());
            buf.extend_from_slice(artifact);
            buf.extend_from_slice(&entry.object_id.to_le_bytes());
            buf.extend_from_slice(&entry.node_key.to_le_bytes());
            buf.extend_from_slice(&entry.generation.to_le_bytes());
            buf.extend_from_slice(&entry.artifact_digest);
            buf.extend_from_slice(&(entry.descriptor.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.descriptor);
        }
        if let Some(key) = *self.cluster_key.lock() {
            buf.push(1);
            buf.extend_from_slice(&self.cluster_key_generation.lock().to_le_bytes());
            buf.extend_from_slice(&key);
        } else {
            buf.push(0);
            buf.extend_from_slice(&0u64.to_le_bytes());
            buf.extend_from_slice(&[0u8; 32]);
        }
        buf
    }

    fn restore_bytes(&self, data: &[u8]) {
        if data.len() < 12 {
            return;
        }
        let magic = u64::from_le_bytes(data[0..8].try_into().ok().unwrap_or_default());
        if magic != CATALOG_MAGIC_V1
            && magic != CATALOG_MAGIC_V2
            && magic != CATALOG_MAGIC_V3
            && magic != CATALOG_MAGIC_V4
            && magic != CATALOG_MAGIC_V5
            && magic != CATALOG_MAGIC_V6
            && magic != CATALOG_MAGIC_V7
        {
            return;
        }
        let count = u32::from_le_bytes(data[8..12].try_into().ok().unwrap_or_default()) as usize;
        let mut pos = 12;
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            let Some((name, after_name)) = take_len_bytes(data, pos) else {
                return;
            };
            let Some((node, after_node)) = take_len_bytes(data, after_name) else {
                return;
            };
            let (generation, after_generation) = if magic != CATALOG_MAGIC_V1 {
                let Some(bytes) = data.get(after_node..after_node.saturating_add(8)) else {
                    return;
                };
                let Ok(bytes) = <[u8; 8]>::try_from(bytes) else {
                    return;
                };
                (u64::from_le_bytes(bytes), after_node + 8)
            } else {
                (1, after_node)
            };
            let (active, after_entry) = if magic == CATALOG_MAGIC_V3
                || magic == CATALOG_MAGIC_V4
                || magic == CATALOG_MAGIC_V5
                || magic == CATALOG_MAGIC_V6
                || magic == CATALOG_MAGIC_V7
            {
                let Some(active) = data.get(after_generation) else {
                    return;
                };
                (*active != 0, after_generation + 1)
            } else {
                (true, after_generation)
            };
            entries.insert(
                name.to_vec(),
                CatalogEntry {
                    node: node.to_vec(),
                    generation,
                    active,
                },
            );
            pos = after_entry;
        }
        *self.entries.lock() = entries;

        let mut deployments = BTreeMap::new();
        if magic == CATALOG_MAGIC_V4
            || magic == CATALOG_MAGIC_V5
            || magic == CATALOG_MAGIC_V6
            || magic == CATALOG_MAGIC_V7
        {
            let Some(bytes) = data.get(pos..pos.saturating_add(4)) else {
                return;
            };
            let Ok(bytes) = <[u8; 4]>::try_from(bytes) else {
                return;
            };
            let deploy_count = u32::from_le_bytes(bytes) as usize;
            pos += 4;
            for _ in 0..deploy_count {
                let Some((artifact, after_artifact)) = take_len_bytes(data, pos) else {
                    return;
                };
                let Some((object_id, after_object)) = read_u64(data, after_artifact) else {
                    return;
                };
                let Some((node_key, after_node)) = read_u64(data, after_object) else {
                    return;
                };
                let Some((generation, after_generation)) = read_u64(data, after_node) else {
                    return;
                };
                // V4 carried a placeholder MAC after the generation; V5 did
                // not. V6 pins the artifact's complete SHA-256 identity.
                let (artifact_digest, after_entry) = if magic == CATALOG_MAGIC_V4 {
                    match read_u64(data, after_generation) {
                        Some((_, after_mac)) => ([0; 32], after_mac),
                        None => return,
                    }
                } else if magic == CATALOG_MAGIC_V6 || magic == CATALOG_MAGIC_V7 {
                    let Some(digest) =
                        data.get(after_generation..after_generation.saturating_add(32))
                    else {
                        return;
                    };
                    let Ok(digest) = <[u8; 32]>::try_from(digest) else {
                        return;
                    };
                    (digest, after_generation + 32)
                } else {
                    ([0; 32], after_generation)
                };
                let (descriptor, after_entry) = if magic == CATALOG_MAGIC_V7 {
                    let Some((descriptor, after_descriptor)) = take_len_bytes(data, after_entry)
                    else {
                        return;
                    };
                    if descriptor.len() > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN {
                        return;
                    }
                    (descriptor.to_vec(), after_descriptor)
                } else {
                    (Vec::new(), after_entry)
                };
                deployments.insert(
                    artifact.to_vec(),
                    DeploymentEntry {
                        object_id,
                        node_key,
                        generation,
                        artifact_digest,
                        descriptor,
                    },
                );
                pos = after_entry;
            }
        }
        *self.deployments.lock() = deployments;

        *self.cluster_key.lock() = None;
        *self.cluster_key_generation.lock() = 0;

        if magic == CATALOG_MAGIC_V5 || magic == CATALOG_MAGIC_V6 || magic == CATALOG_MAGIC_V7 {
            let Some(present) = data.get(pos) else {
                return;
            };
            let (generation, key_start) = if magic == CATALOG_MAGIC_V6 || magic == CATALOG_MAGIC_V7
            {
                let Some((generation, after_generation)) = read_u64(data, pos + 1) else {
                    return;
                };
                (generation, after_generation)
            } else {
                (u64::from(*present != 0), pos + 1)
            };
            let Some(key) = data.get(key_start..key_start + 32) else {
                return;
            };
            if *present != 0 {
                let Ok(key) = <[u8; 32]>::try_from(key) else {
                    return;
                };
                *self.cluster_key.lock() = Some(key);
                *self.cluster_key_generation.lock() = generation.max(1);
            }
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
                self.deployment(artifact).map_or_else(Vec::new, |entry| {
                    let mut result = Vec::with_capacity(60 + entry.descriptor.len());
                    result.extend_from_slice(&entry.generation.to_le_bytes());
                    result.extend_from_slice(&entry.object_id.to_le_bytes());
                    result.extend_from_slice(&entry.node_key.to_le_bytes());
                    result.extend_from_slice(&entry.artifact_digest);
                    result.extend_from_slice(&(entry.descriptor.len() as u32).to_le_bytes());
                    result.extend_from_slice(&entry.descriptor);
                    result
                })
            }
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

/// Decode the deployment record returned by a deployment query.
pub fn decode_deployment_result(bytes: &[u8]) -> Option<DeploymentEntry> {
    if bytes.len() < 56 {
        return None;
    }
    let descriptor = if bytes.len() == 56 {
        Vec::new()
    } else {
        let descriptor_len =
            usize::try_from(u32::from_le_bytes(bytes.get(56..60)?.try_into().ok()?)).ok()?;
        if descriptor_len > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN
            || bytes.len() != 60 + descriptor_len
        {
            return None;
        }
        bytes[60..].to_vec()
    };
    Some(DeploymentEntry {
        generation: u64::from_le_bytes(bytes[0..8].try_into().ok()?),
        object_id: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
        node_key: u64::from_le_bytes(bytes[16..24].try_into().ok()?),
        artifact_digest: bytes[24..56].try_into().ok()?,
        descriptor,
    })
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

/// Encode a key-ceremony command: commit the cluster's Ed25519 public key.
pub fn encode_set_cluster_key(key: &[u8; 32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 32);
    buf.push(CMD_SET_CLUSTER_KEY);
    buf.extend_from_slice(key);
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
