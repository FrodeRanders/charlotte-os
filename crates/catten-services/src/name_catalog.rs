//! Replicated name catalog: the Raft state machine behind the distributed
//! name service.
//!
//! Maps `name -> {node_id, generation}`. Connections are
//! node-local capabilities and cannot be replicated, so only the *location* of
//! a registration is committed; resolving it to a connection stays a local
//! operation on the hosting node.
//!
//! The same state machine also carries the cluster **deployment manifest**:
//! `artifact -> {object_id, node_key, generation, mac}`. A deployment is a
//! cluster decision, so it lives in replicated state next to the name
//! catalog; node-local agents read it and act on the assignments addressed
//! to them.
//!
//! ## Log command encoding
//!
//! ```text
//! register:   0x01 | name_len:u32 | name | node_len:u32 | node
//! unregister: 0x02 | name_len:u32 | name
//! deploy:     0x05 | artifact_len:u32 | artifact | object_id:u64 | node_key:u64 | mac:u64
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
const CATALOG_MAGIC_V1: u64 = 0x4341_5441_4c4f_474d; // "CATALOGM"
const CATALOG_MAGIC_V2: u64 = 0x4341_5441_4c4f_4732; // "CATALOG2"
const CATALOG_MAGIC_V3: u64 = 0x4341_5441_4c4f_4733; // "CATALOG3"
const CATALOG_MAGIC_V4: u64 = 0x4341_5441_4c4f_4734; // "CATALOG4"

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
/// NIC MAC); `mac` is a placeholder cluster signature over the record
/// (FNV-1a keyed with the deployment secret) until real cryptography is
/// introduced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentEntry {
    pub object_id: u64,
    pub node_key: u64,
    pub generation: u64,
    pub mac: u64,
}

pub struct NameCatalog {
    entries: spin::Mutex<BTreeMap<Vec<u8>, CatalogEntry>>,
    deployments: spin::Mutex<BTreeMap<Vec<u8>, DeploymentEntry>>,
    last_apply: spin::Mutex<Option<Vec<u8>>>,
}

impl NameCatalog {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: spin::Mutex::new(BTreeMap::new()),
            deployments: spin::Mutex::new(BTreeMap::new()),
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
                let generation =
                    entries.get(name).map_or(1, |entry| entry.generation.saturating_add(1));
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
                let (mac, _) = read_u64(command, after_node).unwrap_or((0, after_node));
                let mut deployments = self.deployments.lock();
                let generation =
                    deployments.get(artifact).map_or(1, |entry| entry.generation.saturating_add(1));
                deployments.insert(
                    artifact.to_vec(),
                    DeploymentEntry {
                        object_id,
                        node_key,
                        generation,
                        mac,
                    },
                );
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
        // V4 appends the deployment manifest: count + records.
        size += 4;
        for artifact in deployments.keys() {
            size += 4 + artifact.len() + 8 + 8 + 8 + 8;
        }
        let mut buf = Vec::with_capacity(size);
        buf.extend_from_slice(&CATALOG_MAGIC_V4.to_le_bytes());
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
            buf.extend_from_slice(&entry.mac.to_le_bytes());
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
            let (active, after_entry) = if magic == CATALOG_MAGIC_V3 || magic == CATALOG_MAGIC_V4 {
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
        if magic == CATALOG_MAGIC_V4 {
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
                let Some((mac, after_mac)) = read_u64(data, after_generation) else {
                    return;
                };
                deployments.insert(
                    artifact.to_vec(),
                    DeploymentEntry {
                        object_id,
                        node_key,
                        generation,
                        mac,
                    },
                );
                pos = after_mac;
            }
        }
        *self.deployments.lock() = deployments;
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
                    let mut result = Vec::with_capacity(32);
                    result.extend_from_slice(&entry.generation.to_le_bytes());
                    result.extend_from_slice(&entry.object_id.to_le_bytes());
                    result.extend_from_slice(&entry.node_key.to_le_bytes());
                    result.extend_from_slice(&entry.mac.to_le_bytes());
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
    if bytes.len() < 32 {
        return None;
    }
    Some(DeploymentEntry {
        generation: u64::from_le_bytes(bytes[0..8].try_into().ok()?),
        object_id: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
        node_key: u64::from_le_bytes(bytes[16..24].try_into().ok()?),
        mac: u64::from_le_bytes(bytes[24..32].try_into().ok()?),
    })
}

/// Encode a deployment command: assign `artifact` (stored at `object_id`) to
/// the node identified by `node_key`, with the cluster signature `mac`.
pub fn encode_deploy(artifact: &[u8], object_id: u64, node_key: u64, mac: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 4 + artifact.len() + 24);
    buf.push(CMD_DEPLOY);
    buf.extend_from_slice(&(artifact.len() as u32).to_le_bytes());
    buf.extend_from_slice(artifact);
    buf.extend_from_slice(&object_id.to_le_bytes());
    buf.extend_from_slice(&node_key.to_le_bytes());
    buf.extend_from_slice(&mac.to_le_bytes());
    buf
}
