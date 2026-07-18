use alloc::vec::Vec;

use crate::types::LogEntry;

pub trait LogStore {
    fn snapshot_index(&self) -> u64;
    fn snapshot_term(&self) -> u64;
    fn last_index(&self) -> u64;
    fn last_term(&self) -> u64;
    fn term_at(&self, index: u64) -> u64;
    fn entry_at(&self, index: u64) -> Option<LogEntry>;
    fn append(&self, entries: Vec<LogEntry>);
    fn truncate_from(&self, index: u64);
    fn entries_from(&self, index: u64) -> Vec<LogEntry>;
    fn compact_up_to(&self, index: u64);
    fn snapshot_data(&self) -> Vec<u8>;
    fn install_snapshot(
        &self,
        last_included_index: u64,
        last_included_term: u64,
        snapshot_data: Vec<u8>,
    );
}

pub trait PersistentStateStore {
    fn current_term(&self) -> u64;
    fn set_current_term(&self, term: u64);
    fn voted_for(&self) -> Option<alloc::string::String>;
    fn set_voted_for(&self, peer_id: Option<alloc::string::String>);
}

pub struct InMemoryLogStore {
    entries: spin::Mutex<Vec<LogEntry>>,
    snapshot_idx: spin::Mutex<u64>,
    snapshot_term_val: spin::Mutex<u64>,
    snapshot_bytes: spin::Mutex<Vec<u8>>,
}

impl InMemoryLogStore {
    pub fn new() -> Self {
        Self {
            entries: spin::Mutex::new(Vec::new()),
            snapshot_idx: spin::Mutex::new(0),
            snapshot_term_val: spin::Mutex::new(0),
            snapshot_bytes: spin::Mutex::new(Vec::new()),
        }
    }
}

impl LogStore for InMemoryLogStore {
    fn snapshot_index(&self) -> u64 {
        *self.snapshot_idx.lock()
    }

    fn snapshot_term(&self) -> u64 {
        *self.snapshot_term_val.lock()
    }

    fn last_index(&self) -> u64 {
        let entries = self.entries.lock();
        let base = *self.snapshot_idx.lock();
        if entries.is_empty() {
            base
        } else {
            base + entries.len() as u64
        }
    }

    fn last_term(&self) -> u64 {
        let entries = self.entries.lock();
        let _base = *self.snapshot_idx.lock();
        if entries.is_empty() {
            *self.snapshot_term_val.lock()
        } else {
            entries[entries.len() - 1].term
        }
    }

    fn term_at(&self, index: u64) -> u64 {
        let base = *self.snapshot_idx.lock();
        if index == 0 {
            return 0;
        }
        if index == base {
            return *self.snapshot_term_val.lock();
        }
        if index > base {
            let entries = self.entries.lock();
            let offset = (index - base - 1) as usize;
            if offset < entries.len() {
                return entries[offset].term;
            }
        }
        0
    }

    fn entry_at(&self, index: u64) -> Option<LogEntry> {
        let base = *self.snapshot_idx.lock();
        if index <= base {
            return None;
        }
        let entries = self.entries.lock();
        let offset = (index - base - 1) as usize;
        entries.get(offset).cloned()
    }

    fn append(&self, entries: Vec<LogEntry>) {
        self.entries.lock().extend(entries);
    }

    fn truncate_from(&self, index: u64) {
        let base = *self.snapshot_idx.lock();
        if index <= base {
            return;
        }
        let offset = (index - base - 1) as usize;
        let mut entries = self.entries.lock();
        entries.truncate(offset);
    }

    fn entries_from(&self, index: u64) -> Vec<LogEntry> {
        let base = *self.snapshot_idx.lock();
        if index > self.last_index() {
            return Vec::new();
        }
        let offset = if index <= base {
            0
        } else {
            (index - base - 1) as usize
        };
        let entries = self.entries.lock();
        entries[offset..].to_vec()
    }

    fn compact_up_to(&self, index: u64) {
        let base = *self.snapshot_idx.lock();
        if index <= base {
            return;
        }
        let offset = (index - base) as usize;
        let mut entries = self.entries.lock();
        let compacted_term = if offset > 0 && offset <= entries.len() {
            entries[offset - 1].term
        } else {
            return;
        };
        entries.drain(0..offset);
        *self.snapshot_idx.lock() = index;
        *self.snapshot_term_val.lock() = compacted_term;
    }

    fn snapshot_data(&self) -> Vec<u8> {
        self.snapshot_bytes.lock().clone()
    }

    fn install_snapshot(
        &self,
        last_included_index: u64,
        last_included_term: u64,
        snapshot_data: Vec<u8>,
    ) {
        let mut entries = self.entries.lock();
        entries.clear();
        *self.snapshot_idx.lock() = last_included_index;
        *self.snapshot_term_val.lock() = last_included_term;
        *self.snapshot_bytes.lock() = snapshot_data;
    }
}

pub struct InMemoryPersistentStateStore {
    current_term: spin::Mutex<u64>,
    voted_for: spin::Mutex<Option<alloc::string::String>>,
}

impl InMemoryPersistentStateStore {
    pub fn new() -> Self {
        Self {
            current_term: spin::Mutex::new(0),
            voted_for: spin::Mutex::new(None),
        }
    }
}

impl PersistentStateStore for InMemoryPersistentStateStore {
    fn current_term(&self) -> u64 {
        *self.current_term.lock()
    }

    fn set_current_term(&self, term: u64) {
        *self.current_term.lock() = term;
    }

    fn voted_for(&self) -> Option<alloc::string::String> {
        self.voted_for.lock().clone()
    }

    fn set_voted_for(&self, peer_id: Option<alloc::string::String>) {
        *self.voted_for.lock() = peer_id;
    }
}
