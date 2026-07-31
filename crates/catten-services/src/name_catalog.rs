//! Replicated name catalog: the Raft state machine behind the distributed
//! name service.
//!
//! Maps `name -> node_id` (which node registered the name). Connections are
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
const CATALOG_MAGIC: u64 = 0x4341_5441_4c4f_474d; // "CATALOGM"

pub struct NameCatalog {
    entries: spin::Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    last_apply: spin::Mutex<Option<Vec<u8>>>,
}

impl NameCatalog {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: spin::Mutex::new(BTreeMap::new()),
            last_apply: spin::Mutex::new(None),
        })
    }

    /// The node that registered `name`, or `None`.
    pub fn lookup(&self, name: &[u8]) -> Option<Vec<u8>> {
        self.entries.lock().get(name).cloned()
    }

    /// Whether `name` is registered to this node.
    pub fn is_local(&self, name: &[u8], local_node: &[u8]) -> bool {
        self.lookup(name).as_deref() == Some(local_node)
    }

    pub fn registered_count(&self) -> usize {
        self.entries.lock().len()
    }

    /// Snapshot copy of the whole `name -> node` catalog.
    pub fn entries(&self) -> alloc::vec::Vec<(alloc::vec::Vec<u8>, alloc::vec::Vec<u8>)> {
        let entries = self.entries.lock();
        entries
            .iter()
            .map(|(name, node)| (name.clone(), node.clone()))
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
                self.entries.lock().insert(name.to_vec(), node.to_vec());
                node.to_vec()
            }
            Some(CMD_UNREGISTER) => {
                let Some((name, _)) = take_len_bytes(command, 1) else {
                    return Vec::new();
                };
                self.entries.lock().remove(name);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn snapshot_bytes(&self) -> Vec<u8> {
        let entries = self.entries.lock();
        let mut size = 8 + 4; // magic + count
        for (name, node) in entries.iter() {
            size += 4 + name.len() + 4 + node.len();
        }
        let mut buf = Vec::with_capacity(size);
        buf.extend_from_slice(&CATALOG_MAGIC.to_le_bytes());
        buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (name, node) in entries.iter() {
            buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
            buf.extend_from_slice(name);
            buf.extend_from_slice(&(node.len() as u32).to_le_bytes());
            buf.extend_from_slice(node);
        }
        buf
    }

    fn restore_bytes(&self, data: &[u8]) {
        if data.len() < 12 {
            return;
        }
        if u64::from_le_bytes(data[0..8].try_into().ok().unwrap_or_default()) != CATALOG_MAGIC {
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
            entries.insert(name.to_vec(), node.to_vec());
            pos = after_node;
        }
        *self.entries.lock() = entries;
    }
}

impl StateMachine for NameCatalog {
    fn apply(&self, _term: u64, command: &[u8]) {
        let result = self.apply_command(command);
        *self.last_apply.lock() = if result.is_empty() { None } else { Some(result) };
    }

    fn apply_with_result(&self, _term: u64, command: &[u8]) -> Vec<u8> {
        let result = self.apply_command(command);
        *self.last_apply.lock() = if result.is_empty() { None } else { Some(result.clone()) };
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
        self.lookup(query).unwrap_or_default()
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
