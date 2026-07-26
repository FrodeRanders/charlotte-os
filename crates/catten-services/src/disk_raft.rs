//! Disk-backed Raft persistent state and log store.
//!
//! Implements `catten_graft::log_store::{LogStore, PersistentStateStore}`
//! on top of the persistent object store (`"obj"` service). Each store
//! connects to the object store via endpoint IPC and persists data as objects.
//!
//! ## Object ID layout
//!
//! Each Raft node derives a private four-object range from its cluster and
//! node identity. Within that range:
//!
//! - slot 0: persistent state (current_term + voted_for)
//! - slot 1: snapshot metadata (last_included_index + last_included_term)
//! - slot 2: snapshot data blob
//! - slot 3: log entries (all entries serialised as a single blob)
//!
//! ## Persistence model
//!
//! Mutations write the full blob to the object store and call FLUSH before
//! returning. This is simple and correct: a crash between two mutations
//! leaves the store in a consistent state (the last flushed version).
use alloc::string::String;
use alloc::vec::Vec;

use catten_graft::log_store::{LogStore, PersistentStateStore};
use catten_graft::types::LogEntry;
use catten_syscall::*;
use spin::Mutex;

const REPLY_SPINS: u64 = u64::MAX;
const BUFFER_VADDR: usize = 0x0000_0000_0070_0000;

#[derive(Clone, Copy)]
struct ObjectIds {
    state: u64,
    snapshot_meta: u64,
    snapshot_data: u64,
    log: u64,
}

impl ObjectIds {
    fn new(namespace: u64) -> Self {
        // The high bit keeps stable service-owned IDs away from the object
        // store's monotonically allocated low-numbered IDs.
        let base = 0x8000_0000_0000_0000 | ((namespace & 0x1fff_ffff_ffff_ffff) << 2);
        Self {
            state: base,
            snapshot_meta: base + 1,
            snapshot_data: base + 2,
            log: base + 3,
        }
    }
}

fn objstore_connect(ns_conn: u64, wait_for_service: bool) -> Option<u64> {
    let opcode = if wait_for_service {
        crate::ns::OP_LOOKUP
    } else {
        crate::ns::OP_TRY_LOOKUP
    };
    let lookup = ipc_scalar_call_connection(
        ns_conn,
        opcode,
        crate::objstore::NAME,
        0,
        IpcRights::SEND | IpcRights::CALL,
    );
    if lookup == 0 { return None; }
    let (generation, conn) = unsafe { crate::wait_reply(lookup, REPLY_SPINS) };
    if generation < 1 || conn == 0 { return None; }
    Some(conn)
}

fn obj_create_at(obj_conn: u64, object_id: u64) -> bool {
    let call = ipc_scalar_call(
        obj_conn,
        charlotte_protocol_objstore::OP_CREATE_AT,
        object_id,
    );
    if call == 0 {
        return false;
    }
    let (result, _) = unsafe { crate::wait_reply(call, REPLY_SPINS) };
    result == charlotte_protocol_objstore::ERR_OK
        || result == charlotte_protocol_objstore::ERR_EXISTS
}

fn obj_write(obj_conn: u64, object_id: u64, data: &[u8]) -> bool {
    let mem = memory_alloc(1);
    if mem == 0 { return false; }
    if memory_map(mem, BUFFER_VADDR, true) != 0 { memory_close(mem); return false; }
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), BUFFER_VADDR as *mut u8, data.len().min(4096)); }
    memory_unmap(mem);
    let call = ipc_scalar_call_move(obj_conn, crate::objstore::OP_WRITE, object_id, mem);
    if call == 0 { return false; }
    let (result, _) = unsafe { crate::wait_reply(call, REPLY_SPINS) };
    result == 0
}

fn obj_read(obj_conn: u64, object_id: u64) -> Option<Vec<u8>> {
    let call = ipc_scalar_call(obj_conn, crate::objstore::OP_READ, object_id);
    if call == 0 {
        return None;
    }
    let (status, result, returned_connection, memory) = ipc_reply_wait_with_memory(call);
    ipc_close(call);
    if returned_connection != 0 {
        ipc_close(returned_connection);
    }
    if status != 0 || result != 0 || memory == 0 {
        if memory != 0 {
            memory_close(memory);
        }
        return None;
    }
    if memory_map(memory, BUFFER_VADDR, false) != 0 {
        memory_close(memory);
        return None;
    }
    let mut buf = alloc::vec![0u8; 4096];
    unsafe { core::ptr::copy_nonoverlapping(BUFFER_VADDR as *const u8, buf.as_mut_ptr(), 4096); }
    memory_unmap(memory);
    memory_close(memory);
    Some(buf)
}

fn obj_flush(obj_conn: u64) -> bool {
    let call = ipc_scalar_call_connection(obj_conn, crate::objstore::OP_FLUSH, 0, 0, IpcRights::SEND | IpcRights::CALL);
    if call == 0 { return false; }
    let (result, _) = unsafe { crate::wait_reply(call, REPLY_SPINS) };
    result == 0
}

fn serialize_entry(entry: &LogEntry) -> Vec<u8> {
    let peer_bytes = entry.peer_id.as_bytes();
    let peer_len = (peer_bytes.len() as u32).min(255);
    let data_len = entry.data.len() as u32;
    let mut buf = alloc::vec![0u8; 16 + peer_len as usize + data_len as usize];
    buf[0..8].copy_from_slice(&entry.term.to_le_bytes());
    buf[8..12].copy_from_slice(&peer_len.to_le_bytes());
    buf[12..16].copy_from_slice(&data_len.to_le_bytes());
    buf[16..16 + peer_len as usize].copy_from_slice(&peer_bytes[..peer_len as usize]);
    buf[16 + peer_len as usize..].copy_from_slice(&entry.data);
    buf
}

fn deserialize_entries(buf: &[u8]) -> Vec<LogEntry> {
    let mut entries = Vec::new();
    let mut pos = 0;
    while pos + 16 <= buf.len() {
        let term = u64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap_or([0; 8]));
        let peer_len = u32::from_le_bytes(buf[pos + 8..pos + 12].try_into().unwrap_or([0; 4])) as usize;
        let data_len = u32::from_le_bytes(buf[pos + 12..pos + 16].try_into().unwrap_or([0; 4])) as usize;
        if peer_len > 255 || term == 0 && peer_len == 0 && data_len == 0 { break; }
        if pos + 16 + peer_len + data_len > buf.len() { break; }
        let peer_id = core::str::from_utf8(&buf[pos + 16..pos + 16 + peer_len])
            .ok()
            .map(String::from)
            .unwrap_or_default();
        let data = buf[pos + 16 + peer_len..pos + 16 + peer_len + data_len].to_vec();
        entries.push(LogEntry { term, peer_id, data });
        pos += 16 + peer_len + data_len;
    }
    entries
}

fn serialize_entries(entries: &[LogEntry]) -> Vec<u8> {
    let mut buf = Vec::new();
    for e in entries {
        let s = serialize_entry(e);
        buf.extend_from_slice(&s);
    }
    buf
}

// ---------------------------------------------------------------------------
// PersistentStateStore
// ---------------------------------------------------------------------------

pub struct DiskPersistentStateStore {
    obj_conn: u64,
    object_id: u64,
    current_term: Mutex<u64>,
    voted_for: Mutex<Option<String>>,
}

impl DiskPersistentStateStore {
    pub fn new(ns_conn: u64, namespace: u64, wait_for_service: bool) -> Option<Self> {
        let obj_conn = objstore_connect(ns_conn, wait_for_service)?;
        let object_id = ObjectIds::new(namespace).state;
        if !obj_create_at(obj_conn, object_id) {
            return None;
        }
        let (current_term, voted_for) = if let Some(buf) = obj_read(obj_conn, object_id) {
            if buf.len() < 8 { (0, None) }
            else {
                let term = u64::from_le_bytes(buf[0..8].try_into().unwrap_or([0; 8]));
                let vf_len = u32::from_le_bytes(buf[8..12].try_into().unwrap_or([0; 4])) as usize;
                let vf = if vf_len > 0 && vf_len <= 256 && 12 + vf_len <= buf.len() {
                    core::str::from_utf8(&buf[12..12 + vf_len]).ok().map(String::from)
                } else { None };
                (term, vf)
            }
        } else {
            (0, None)
        };
        Some(Self {
            obj_conn,
            object_id,
            current_term: Mutex::new(current_term),
            voted_for: Mutex::new(voted_for),
        })
    }

    fn persist(&self) {
        let term = *self.current_term.lock();
        let vf = self.voted_for.lock();
        let vf_bytes = vf.as_ref().map(|s| s.as_bytes()).unwrap_or(&[]);
        let vf_len = (vf_bytes.len() as u32).min(256);
        let mut data = alloc::vec![0u8; 12 + vf_len as usize];
        data[0..8].copy_from_slice(&term.to_le_bytes());
        data[8..12].copy_from_slice(&vf_len.to_le_bytes());
        if vf_len > 0 {
            data[12..].copy_from_slice(&vf_bytes[..vf_len as usize]);
        }
        assert!(
            obj_write(self.obj_conn, self.object_id, &data) && obj_flush(self.obj_conn),
            "failed to persist Raft term/vote state"
        );
    }
}

impl PersistentStateStore for DiskPersistentStateStore {
    fn current_term(&self) -> u64 { *self.current_term.lock() }

    fn set_current_term(&self, term: u64) {
        *self.current_term.lock() = term;
        self.persist();
    }

    fn voted_for(&self) -> Option<String> { self.voted_for.lock().clone() }

    fn set_voted_for(&self, peer_id: Option<String>) {
        *self.voted_for.lock() = peer_id;
        self.persist();
    }
}

// ---------------------------------------------------------------------------
// LogStore
// ---------------------------------------------------------------------------

pub struct DiskLogStore {
    obj_conn: u64,
    objects: ObjectIds,
    entries: Mutex<Vec<LogEntry>>,
    snapshot_idx: Mutex<u64>,
    snapshot_term: Mutex<u64>,
    snapshot_data: Mutex<Vec<u8>>,
}

impl DiskLogStore {
    pub fn new(ns_conn: u64, namespace: u64, wait_for_service: bool) -> Option<Self> {
        let obj_conn = objstore_connect(ns_conn, wait_for_service)?;
        let objects = ObjectIds::new(namespace);
        if !obj_create_at(obj_conn, objects.snapshot_meta)
            || !obj_create_at(obj_conn, objects.snapshot_data)
            || !obj_create_at(obj_conn, objects.log)
        {
            return None;
        }

        let (snapshot_idx, snapshot_term) =
            if let Some(buf) = obj_read(obj_conn, objects.snapshot_meta) {
            if buf.len() < 16 { (0, 0) }
            else {
                (u64::from_le_bytes(buf[0..8].try_into().unwrap_or([0; 8])),
                 u64::from_le_bytes(buf[8..16].try_into().unwrap_or([0; 8])))
            }
        } else { (0, 0) };

        let snapshot_data = obj_read(obj_conn, objects.snapshot_data).unwrap_or_default();

        let entries = if let Some(buf) = obj_read(obj_conn, objects.log) {
            deserialize_entries(&buf)
        } else {
            Vec::new()
        };

        Some(Self {
            obj_conn,
            objects,
            entries: Mutex::new(entries),
            snapshot_idx: Mutex::new(snapshot_idx),
            snapshot_term: Mutex::new(snapshot_term),
            snapshot_data: Mutex::new(snapshot_data),
        })
    }

    fn persist_log(&self) {
        let entries = self.entries.lock();
        let data = serialize_entries(&entries);
        assert!(
            obj_write(self.obj_conn, self.objects.log, &data) && obj_flush(self.obj_conn),
            "failed to persist Raft log"
        );
    }
}

impl LogStore for DiskLogStore {
    fn snapshot_index(&self) -> u64 { *self.snapshot_idx.lock() }
    fn snapshot_term(&self) -> u64 { *self.snapshot_term.lock() }

    fn last_index(&self) -> u64 {
        let base = *self.snapshot_idx.lock();
        let entries = self.entries.lock();
        if entries.is_empty() { base } else { base + entries.len() as u64 }
    }

    fn last_term(&self) -> u64 {
        let entries = self.entries.lock();
        if entries.is_empty() { *self.snapshot_term.lock() } else { entries[entries.len() - 1].term }
    }

    fn term_at(&self, index: u64) -> u64 {
        let base = *self.snapshot_idx.lock();
        if index == 0 { return 0; }
        if index == base { return *self.snapshot_term.lock(); }
        if index > base {
            let entries = self.entries.lock();
            let offset = (index - base - 1) as usize;
            if offset < entries.len() { return entries[offset].term; }
        }
        0
    }

    fn entry_at(&self, index: u64) -> Option<LogEntry> {
        let base = *self.snapshot_idx.lock();
        if index <= base { return None; }
        let entries = self.entries.lock();
        let offset = (index - base - 1) as usize;
        entries.get(offset).cloned()
    }

    fn append(&self, new_entries: Vec<LogEntry>) {
        self.entries.lock().extend(new_entries);
        self.persist_log();
    }

    fn truncate_from(&self, index: u64) {
        let base = *self.snapshot_idx.lock();
        if index <= base { return; }
        let offset = (index - base - 1) as usize;
        self.entries.lock().truncate(offset);
        self.persist_log();
    }

    fn entries_from(&self, index: u64) -> Vec<LogEntry> {
        let base = *self.snapshot_idx.lock();
        let entries = self.entries.lock();
        let last = if entries.is_empty() { base } else { base + entries.len() as u64 };
        if index > last { return Vec::new(); }
        let offset = if index <= base { 0 } else { (index - base - 1) as usize };
        entries[offset..].to_vec()
    }

    fn compact_up_to(&self, index: u64) {
        let base = *self.snapshot_idx.lock();
        if index <= base { return; }
        let offset = (index - base) as usize;
        let mut entries = self.entries.lock();
        if offset == 0 || offset > entries.len() { return; }
        let compacted_term = entries[offset - 1].term;
        entries.drain(0..offset);
        *self.snapshot_idx.lock() = index;
        *self.snapshot_term.lock() = compacted_term;
        let mut meta = [0u8; 16];
        meta[0..8].copy_from_slice(&index.to_le_bytes());
        meta[8..16].copy_from_slice(&compacted_term.to_le_bytes());
        assert!(
            obj_write(self.obj_conn, self.objects.snapshot_meta, &meta),
            "failed to persist Raft snapshot metadata"
        );
        self.persist_log();
    }

    fn snapshot_data(&self) -> Vec<u8> { self.snapshot_data.lock().clone() }

    fn install_snapshot(&self, index: u64, term: u64, data: Vec<u8>) {
        self.entries.lock().clear();
        *self.snapshot_idx.lock() = index;
        *self.snapshot_term.lock() = term;
        *self.snapshot_data.lock() = data.clone();
        let mut meta = [0u8; 16];
        meta[0..8].copy_from_slice(&index.to_le_bytes());
        meta[8..16].copy_from_slice(&term.to_le_bytes());
        assert!(
            obj_write(self.obj_conn, self.objects.snapshot_meta, &meta)
                && obj_write(self.obj_conn, self.objects.snapshot_data, &data),
            "failed to persist Raft snapshot"
        );
        self.persist_log();
    }
}
