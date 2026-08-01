//! Replicated name catalog: the Raft state machine behind the distributed
//! name service.
//!
//! Maps `name -> {node_id, generation}`. Connections are
//! node-local capabilities and cannot be replicated, so only the *location* of
//! a registration is committed; resolving it to a connection stays a local
//! operation on the hosting node.
//!
//! ## Log command encoding
//!
//! ```text
//! register:   0x01 | name_len:u32 | name | node_len:u32 | node
//! unregister: 0x02 | name_len:u32 | name
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
const CATALOG_MAGIC_V1: u64 = 0x4341_5441_4c4f_474d; // "CATALOGM"
const CATALOG_MAGIC_V2: u64 = 0x4341_5441_4c4f_4732; // "CATALOG2"
const CATALOG_MAGIC_V3: u64 = 0x4341_5441_4c4f_4733; // "CATALOG3"

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    pub node: Vec<u8>,
    pub generation: u64,
    pub active: bool,
}

pub struct NameCatalog {
    entries: spin::Mutex<BTreeMap<Vec<u8>, CatalogEntry>>,
    last_apply: spin::Mutex<Option<Vec<u8>>>,
}

impl NameCatalog {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: spin::Mutex::new(BTreeMap::new()),
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

    /// Whether `name` is registered to this node.
    pub fn is_local(&self, name: &[u8], local_node: &[u8]) -> bool {
        self.lookup(name).is_some_and(|entry| entry.node == local_node)
    }

    pub fn registered_count(&self) -> usize {
        self.entries.lock().values().filter(|entry| entry.active && !entry.node.is_empty()).count()
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
            _ => Vec::new(),
        }
    }

    fn snapshot_bytes(&self) -> Vec<u8> {
        let entries = self.entries.lock();
        let mut size = 8 + 4; // magic + count
        for (name, entry) in entries.iter() {
            size += 4 + name.len() + 4 + entry.node.len() + 8 + 1;
        }
        let mut buf = Vec::with_capacity(size);
        buf.extend_from_slice(&CATALOG_MAGIC_V3.to_le_bytes());
        buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (name, entry) in entries.iter() {
            buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
            buf.extend_from_slice(name);
            buf.extend_from_slice(&(entry.node.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.node);
            buf.extend_from_slice(&entry.generation.to_le_bytes());
            buf.push(u8::from(entry.active));
        }
        buf
    }

    fn restore_bytes(&self, data: &[u8]) {
        if data.len() < 12 {
            return;
        }
        let magic = u64::from_le_bytes(data[0..8].try_into().ok().unwrap_or_default());
        if magic != CATALOG_MAGIC_V1 && magic != CATALOG_MAGIC_V2 && magic != CATALOG_MAGIC_V3 {
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
            let (active, after_entry) = if magic == CATALOG_MAGIC_V3 {
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
        self.lookup(query).map_or_else(Vec::new, |entry| {
            let mut result = Vec::with_capacity(8 + entry.node.len());
            result.extend_from_slice(&entry.generation.to_le_bytes());
            result.extend_from_slice(&entry.node);
            result
        })
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
