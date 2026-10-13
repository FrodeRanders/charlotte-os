//! Current-only object-store-backed Raft persistence.
//!
//! One jointly opened pair retains tagged term/vote/join state and an atomic
//! snapshot/log record. Only two absent objects authorize initialization;
//! incomplete, empty, retired or unsupported records never reset a voter.
//! Each mutation writes its complete record and flushes before returning.

use alloc::{
    string::String,
    sync::Arc,
    vec::Vec,
};

use catten_graft::{
    log_store::{
        JoinAdmission,
        LogStore,
        PersistentStateStore,
    },
    types::LogEntry,
};
use catten_rt::owned::ConnectionRef;
use spin::Mutex;

use crate::objstore_client::{
    Client,
    ObjectStore,
};

const STATE_MAGIC: &[u8; 8] = b"CRFTSTA3";
const STATE_HEADER_LEN: usize = 32;
const LOG_STATE_MAGIC: &[u8; 8] = b"CRFTLOG3";
const LOG_STATE_HEADER_LEN: usize = 40;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordKind {
    State,
    Log,
}

/// Opening never repairs or overwrites an existing rejected record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError {
    Io,
    IncompletePair,
    UnsupportedFormat(RecordKind),
    MalformedRecord(RecordKind),
}

#[derive(Clone, Copy)]
struct ObjectIds {
    state: u64,
    log: u64,
}

impl ObjectIds {
    fn new(namespace: u64) -> Self {
        // Compact two-object range in the service-owned 0x8xxx partition.
        // Identity (0x9xxx), artifact (0xfffe) and executable (0xffff) IDs
        // remain disjoint. This replaces the retired four-slot layout.
        let base = 0x8000_0000_0000_0000 | (namespace & 0x0fff_ffff_ffff_fffe);
        Self {
            state: base,
            log: base + 1,
        }
    }
}

#[derive(Default, Debug, Eq, PartialEq)]
struct PersistentState {
    term: u64,
    voted_for: Option<String>,
    admission: Option<JoinAdmission>,
}

#[derive(Default, Debug, Eq, PartialEq)]
struct LogState {
    snapshot_index: u64,
    snapshot_term: u64,
    snapshot: Vec<u8>,
    entries: Vec<LogEntry>,
}

fn copy_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut result = Vec::new();
    result.try_reserve_exact(bytes.len()).ok()?;
    result.extend_from_slice(bytes);
    Some(result)
}

fn owned_id(bytes: &[u8]) -> Option<String> {
    core::str::from_utf8(bytes).ok()?;
    String::from_utf8(copy_bytes(bytes)?).ok()
}

fn serialize_persistent_state(state: &PersistentState) -> Option<Vec<u8>> {
    let vote = state.voted_for.as_deref().unwrap_or("").as_bytes();
    let anchor = state.admission.as_ref().map(|a| a.anchor_id.as_bytes()).unwrap_or(&[]);
    if vote.len() > 256
        || anchor.len() > 256
        || state.voted_for.is_some() && vote.is_empty()
        || state.admission.is_some() && anchor.is_empty()
    {
        return None;
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(STATE_HEADER_LEN + vote.len() + anchor.len()).ok()?;
    bytes.extend_from_slice(STATE_MAGIC);
    bytes.extend_from_slice(&state.term.to_le_bytes());
    bytes.extend_from_slice(&(vote.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(anchor.len() as u32).to_le_bytes());
    bytes.extend_from_slice(
        &state.admission.as_ref().map(|a| a.snapshot_index).unwrap_or(0).to_le_bytes(),
    );
    bytes.extend_from_slice(vote);
    bytes.extend_from_slice(anchor);
    Some(bytes)
}

fn check_magic(bytes: &[u8], magic: &[u8; 8], kind: RecordKind) -> Result<(), StoreError> {
    let actual = bytes.get(..8).ok_or(StoreError::MalformedRecord(kind))?;
    if actual != magic {
        return Err(StoreError::UnsupportedFormat(kind));
    }
    Ok(())
}

fn decode_persistent_state(bytes: &[u8]) -> Option<PersistentState> {
    if bytes.get(..8)? != STATE_MAGIC {
        return None;
    }
    let term = u64::from_le_bytes(bytes.get(8..16)?.try_into().ok()?);
    let vote_len = u32::from_le_bytes(bytes.get(16..20)?.try_into().ok()?) as usize;
    let anchor_len = u32::from_le_bytes(bytes.get(20..24)?.try_into().ok()?) as usize;
    let snapshot_index = u64::from_le_bytes(bytes.get(24..32)?.try_into().ok()?);
    if vote_len > 256 || anchor_len > 256 || anchor_len == 0 && snapshot_index != 0 {
        return None;
    }
    let vote_end = STATE_HEADER_LEN.checked_add(vote_len)?;
    if vote_end.checked_add(anchor_len)? != bytes.len() {
        return None;
    }
    Some(PersistentState {
        term,
        voted_for: if vote_len == 0 {
            None
        } else {
            Some(owned_id(bytes.get(STATE_HEADER_LEN..vote_end)?)?)
        },
        admission: if anchor_len == 0 {
            None
        } else {
            Some(JoinAdmission {
                anchor_id: owned_id(bytes.get(vote_end..)?)?,
                snapshot_index,
            })
        },
    })
}

fn serialize_log_state(state: &LogState) -> Option<Vec<u8>> {
    state.snapshot_index.checked_add(u64::try_from(state.entries.len()).ok()?)?;
    let mut entries_len = 0usize;
    for entry in &state.entries {
        if entry.peer_id.len() > 255
            || entry.term == 0 && entry.peer_id.is_empty() && entry.data.is_empty()
        {
            return None;
        }
        u32::try_from(entry.data.len()).ok()?;
        entries_len = entries_len
            .checked_add(16)?
            .checked_add(entry.peer_id.len())?
            .checked_add(entry.data.len())?;
    }
    let length =
        LOG_STATE_HEADER_LEN.checked_add(state.snapshot.len())?.checked_add(entries_len)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).ok()?;
    bytes.extend_from_slice(LOG_STATE_MAGIC);
    bytes.extend_from_slice(&state.snapshot_index.to_le_bytes());
    bytes.extend_from_slice(&state.snapshot_term.to_le_bytes());
    bytes.extend_from_slice(&(state.snapshot.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(entries_len as u64).to_le_bytes());
    bytes.extend_from_slice(&state.snapshot);
    for entry in &state.entries {
        bytes.extend_from_slice(&entry.term.to_le_bytes());
        bytes.extend_from_slice(&(entry.peer_id.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(entry.data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(entry.peer_id.as_bytes());
        bytes.extend_from_slice(&entry.data);
    }
    Some(bytes)
}

fn decode_log_state(bytes: &[u8]) -> Option<LogState> {
    if bytes.get(..8)? != LOG_STATE_MAGIC {
        return None;
    }
    let snapshot_index = u64::from_le_bytes(bytes.get(8..16)?.try_into().ok()?);
    let snapshot_term = u64::from_le_bytes(bytes.get(16..24)?.try_into().ok()?);
    let snapshot_len =
        usize::try_from(u64::from_le_bytes(bytes.get(24..32)?.try_into().ok()?)).ok()?;
    let entries_len =
        usize::try_from(u64::from_le_bytes(bytes.get(32..40)?.try_into().ok()?)).ok()?;
    let snapshot_end = LOG_STATE_HEADER_LEN.checked_add(snapshot_len)?;
    if snapshot_end.checked_add(entries_len)? != bytes.len() {
        return None;
    }
    let snapshot = copy_bytes(bytes.get(LOG_STATE_HEADER_LEN..snapshot_end)?)?;
    let mut entries = Vec::new();
    let mut pos = snapshot_end;
    while pos < bytes.len() {
        let header = bytes.get(pos..pos.checked_add(16)?)?;
        let term = u64::from_le_bytes(header[..8].try_into().ok()?);
        let peer_len = u32::from_le_bytes(header[8..12].try_into().ok()?) as usize;
        let data_len = u32::from_le_bytes(header[12..16].try_into().ok()?) as usize;
        if peer_len > 255 || term == 0 && peer_len == 0 && data_len == 0 {
            return None;
        }
        let peer_start = pos.checked_add(16)?;
        let data_start = peer_start.checked_add(peer_len)?;
        let end = data_start.checked_add(data_len)?;
        let peer_id = owned_id(bytes.get(peer_start..data_start)?)?;
        let data = copy_bytes(bytes.get(data_start..end)?)?;
        entries.try_reserve(1).ok()?;
        entries.push(LogEntry {
            term,
            peer_id,
            data,
        });
        pos = end;
    }
    snapshot_index.checked_add(u64::try_from(entries.len()).ok()?)?;
    Some(LogState {
        snapshot_index,
        snapshot_term,
        snapshot,
        entries,
    })
}

/// Open one current persistence pair through a uniquely owned connection.
/// The namespace uses two stable objects. Only exact successful creation of
/// both absent records authorizes initialization; any partial failure retains
/// the incomplete objects and returns an error, without reset or retry.
pub fn open(
    names: ConnectionRef<'_>,
    namespace: u64,
) -> Result<(DiskLogStore, DiskPersistentStateStore), StoreError> {
    let client = Client::connect(names).map_err(|_| StoreError::Io)?;
    open_from(Arc::new(client), namespace)
}

fn open_from(
    store: Arc<dyn ObjectStore>,
    namespace: u64,
) -> Result<(DiskLogStore, DiskPersistentStateStore), StoreError> {
    let ids = ObjectIds::new(namespace);
    let state_bytes = store.read(ids.state).map_err(|_| StoreError::Io)?;
    let log_bytes = store.read(ids.log).map_err(|_| StoreError::Io)?;
    let (state, log) = match (state_bytes, log_bytes) {
        (None, None) => {
            let state = PersistentState::default();
            let log = LogState::default();
            let state_bytes = serialize_persistent_state(&state)
                .ok_or(StoreError::MalformedRecord(RecordKind::State))?;
            let log_bytes =
                serialize_log_state(&log).ok_or(StoreError::MalformedRecord(RecordKind::Log))?;
            store.create(ids.state).map_err(|_| StoreError::Io)?;
            store.create(ids.log).map_err(|_| StoreError::Io)?;
            store.write(ids.state, &state_bytes).map_err(|_| StoreError::Io)?;
            store.write(ids.log, &log_bytes).map_err(|_| StoreError::Io)?;
            store.flush().map_err(|_| StoreError::Io)?;
            (state, log)
        }
        (Some(state), Some(log)) => {
            check_magic(&state, STATE_MAGIC, RecordKind::State)?;
            check_magic(&log, LOG_STATE_MAGIC, RecordKind::Log)?;
            let state = decode_persistent_state(&state)
                .ok_or(StoreError::MalformedRecord(RecordKind::State))?;
            let log = decode_log_state(&log).ok_or(StoreError::MalformedRecord(RecordKind::Log))?;
            (state, log)
        }
        _ => return Err(StoreError::IncompletePair),
    };
    Ok((
        DiskLogStore {
            store: store.clone(),
            object_id: ids.log,
            state: Mutex::new(log),
        },
        DiskPersistentStateStore {
            store,
            object_id: ids.state,
            state: Mutex::new(state),
        },
    ))
}

fn persist_fail_stop(what: &str) -> ! {
    catten_rt::logln!("[raft-store] FATAL: failed to persist {}", what);
    unsafe { catten_syscall::thread_exit() }
}

pub struct DiskPersistentStateStore {
    store: Arc<dyn ObjectStore>,
    object_id: u64,
    state: Mutex<PersistentState>,
}

impl DiskPersistentStateStore {
    fn persist(&self) {
        let data = { serialize_persistent_state(&self.state.lock()) };
        let data = data.unwrap_or_else(|| persist_fail_stop("Raft state encoding"));
        if self.store.write(self.object_id, &data).and_then(|()| self.store.flush()).is_err() {
            persist_fail_stop("Raft term/vote/admission state");
        }
    }
}

impl PersistentStateStore for DiskPersistentStateStore {
    fn current_term(&self) -> u64 {
        self.state.lock().term
    }

    fn set_current_term(&self, term: u64) {
        self.state.lock().term = term;
        self.persist();
    }

    fn voted_for(&self) -> Option<String> {
        self.state.lock().voted_for.clone()
    }

    fn set_voted_for(&self, peer_id: Option<String>) {
        self.state.lock().voted_for = peer_id;
        self.persist();
    }

    fn join_admission(&self) -> Option<JoinAdmission> {
        self.state.lock().admission.clone()
    }

    fn set_join_admission(&self, admission: Option<JoinAdmission>) {
        self.state.lock().admission = admission;
        self.persist();
    }
}

pub struct DiskLogStore {
    store: Arc<dyn ObjectStore>,
    object_id: u64,
    state: Mutex<LogState>,
}

impl DiskLogStore {
    fn persist_log_state(&self) {
        let data = { serialize_log_state(&self.state.lock()) };
        let data = data.unwrap_or_else(|| persist_fail_stop("Raft log encoding"));
        if self.store.write(self.object_id, &data).and_then(|()| self.store.flush()).is_err() {
            persist_fail_stop("Raft log state");
        }
    }
}

impl LogStore for DiskLogStore {
    fn reset(&self) {
        *self.state.lock() = LogState::default();
        self.persist_log_state();
    }

    fn snapshot_index(&self) -> u64 {
        self.state.lock().snapshot_index
    }

    fn snapshot_term(&self) -> u64 {
        self.state.lock().snapshot_term
    }

    fn last_index(&self) -> u64 {
        let s = self.state.lock();
        s.snapshot_index + s.entries.len() as u64
    }

    fn last_term(&self) -> u64 {
        let s = self.state.lock();
        s.entries.last().map(|e| e.term).unwrap_or(s.snapshot_term)
    }

    fn term_at(&self, index: u64) -> u64 {
        let s = self.state.lock();
        if index == 0 {
            0
        } else if index == s.snapshot_index {
            s.snapshot_term
        } else if index > s.snapshot_index {
            s.entries.get((index - s.snapshot_index - 1) as usize).map(|e| e.term).unwrap_or(0)
        } else {
            0
        }
    }

    fn entry_at(&self, index: u64) -> Option<LogEntry> {
        let s = self.state.lock();
        if index <= s.snapshot_index {
            None
        } else {
            s.entries.get((index - s.snapshot_index - 1) as usize).cloned()
        }
    }

    fn append(&self, entries: Vec<LogEntry>) {
        self.state.lock().entries.extend(entries);
        self.persist_log_state();
    }

    fn truncate_from(&self, index: u64) {
        {
            let mut s = self.state.lock();
            if index <= s.snapshot_index {
                return;
            }
            let offset = (index - s.snapshot_index - 1) as usize;
            s.entries.truncate(offset);
        }
        self.persist_log_state();
    }

    fn entries_from(&self, index: u64) -> Vec<LogEntry> {
        let s = self.state.lock();
        if index > s.snapshot_index + s.entries.len() as u64 {
            return Vec::new();
        }
        let offset = if index <= s.snapshot_index {
            0
        } else {
            (index - s.snapshot_index - 1) as usize
        };
        s.entries[offset..].to_vec()
    }

    fn snapshot_data(&self) -> Vec<u8> {
        self.state.lock().snapshot.clone()
    }

    fn install_snapshot(&self, index: u64, term: u64, data: Vec<u8>) {
        {
            let mut s = self.state.lock();
            let retain_from = if index == s.snapshot_index && term == s.snapshot_term {
                Some(0)
            } else if index > s.snapshot_index {
                let offset = (index - s.snapshot_index - 1) as usize;
                (offset < s.entries.len() && s.entries[offset].term == term).then_some(offset + 1)
            } else {
                None
            };
            if let Some(offset) = retain_from {
                s.entries.drain(..offset);
            } else {
                s.entries.clear();
            }
            s.snapshot_index = index;
            s.snapshot_term = term;
            s.snapshot = data;
        }
        self.persist_log_state();
    }
}

#[cfg(test)]
mod tests {
    use alloc::{
        collections::BTreeMap,
        vec,
    };

    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Op {
        Read(u64),
        Create(u64),
        Write(u64),
        Flush,
    }

    #[derive(Default)]
    struct MemoryObjects {
        objects: Mutex<BTreeMap<u64, Vec<u8>>>,
        operations: Mutex<Vec<Op>>,
        fail_at: Option<usize>,
        creation_conflict: bool,
    }

    impl MemoryObjects {
        fn record(&self, op: Op) -> Result<(), ()> {
            let mut operations = self.operations.lock();
            let index = operations.len();
            operations.push(op);
            if self.fail_at == Some(index) {
                Err(())
            } else {
                Ok(())
            }
        }
    }

    impl ObjectStore for MemoryObjects {
        fn read(&self, object: u64) -> Result<Option<Vec<u8>>, ()> {
            self.record(Op::Read(object))?;
            Ok(self.objects.lock().get(&object).cloned())
        }

        fn create(&self, object: u64) -> Result<(), ()> {
            self.record(Op::Create(object))?;
            let mut objects = self.objects.lock();
            if self.creation_conflict {
                objects.insert(object, b"other-owner".to_vec());
                return Err(());
            }
            if objects.contains_key(&object) {
                return Err(());
            }
            objects.insert(object, Vec::new());
            Ok(())
        }

        fn write(&self, object: u64, bytes: &[u8]) -> Result<(), ()> {
            self.record(Op::Write(object))?;
            *self.objects.lock().get_mut(&object).ok_or(())? = bytes.to_vec();
            Ok(())
        }

        fn flush(&self) -> Result<(), ()> {
            self.record(Op::Flush)
        }
    }

    fn current_pair() -> Arc<MemoryObjects> {
        let store = Arc::new(MemoryObjects::default());
        open_from(store.clone(), 10).unwrap();
        store.operations.lock().clear();
        store
    }

    #[test]
    fn fresh_pair_is_initialized_once_and_reopened_without_writes() {
        let store = Arc::new(MemoryObjects::default());
        let (log, state) = open_from(store.clone(), 10).unwrap();
        assert_eq!(state.current_term(), 0);
        assert_eq!(log.last_index(), 0);
        let ids = ObjectIds::new(10);
        assert_eq!(ids.log, ids.state + 1);
        assert_eq!(
            *store.operations.lock(),
            vec![
                Op::Read(ids.state),
                Op::Read(ids.log),
                Op::Create(ids.state),
                Op::Create(ids.log),
                Op::Write(ids.state),
                Op::Write(ids.log),
                Op::Flush
            ]
        );
        store.operations.lock().clear();
        drop((log, state));
        open_from(store.clone(), 10).unwrap();
        assert_eq!(*store.operations.lock(), vec![Op::Read(ids.state), Op::Read(ids.log)]);
        assert_eq!(store.objects.lock().len(), 2);
    }

    #[test]
    fn every_open_io_failure_stops_without_replay_or_reset() {
        for fail_at in 0..7 {
            let store = Arc::new(MemoryObjects {
                fail_at: Some(fail_at),
                ..MemoryObjects::default()
            });
            assert!(matches!(open_from(store.clone(), 10), Err(StoreError::Io)));
            assert_eq!(store.operations.lock().len(), fail_at + 1);
        }
        let store = Arc::new(MemoryObjects {
            creation_conflict: true,
            ..MemoryObjects::default()
        });
        assert!(matches!(open_from(store.clone(), 10), Err(StoreError::Io)));
        assert_eq!(store.operations.lock().len(), 3);
        assert_eq!(store.objects.lock()[&ObjectIds::new(10).state], b"other-owner");
    }

    #[test]
    fn incomplete_empty_retired_unknown_and_corrupt_pairs_are_never_repaired() {
        let ids = ObjectIds::new(10);
        for object in [ids.state, ids.log] {
            let store = current_pair();
            store.objects.lock().remove(&object);
            let before = store.objects.lock().clone();
            assert!(matches!(open_from(store.clone(), 10), Err(StoreError::IncompletePair)));
            assert_eq!(*store.objects.lock(), before);
            assert_eq!(store.operations.lock().len(), 2);
        }
        for (object, bytes, expected) in [
            (ids.state, Vec::new(), StoreError::MalformedRecord(RecordKind::State)),
            (ids.log, Vec::new(), StoreError::MalformedRecord(RecordKind::Log)),
            (ids.state, vec![0; 12], StoreError::UnsupportedFormat(RecordKind::State)),
            (ids.state, vec![0; 24], StoreError::UnsupportedFormat(RecordKind::State)),
            (ids.log, vec![0; 16], StoreError::UnsupportedFormat(RecordKind::Log)),
            (ids.state, b"CRFTSTA1".to_vec(), StoreError::UnsupportedFormat(RecordKind::State)),
            (ids.log, b"CRFTLOG1".to_vec(), StoreError::UnsupportedFormat(RecordKind::Log)),
            (ids.state, b"CRFTSTA2".to_vec(), StoreError::UnsupportedFormat(RecordKind::State)),
            (ids.log, b"CRFTLOG2".to_vec(), StoreError::UnsupportedFormat(RecordKind::Log)),
            (ids.log, LOG_STATE_MAGIC.to_vec(), StoreError::MalformedRecord(RecordKind::Log)),
        ] {
            let store = current_pair();
            store.objects.lock().insert(object, bytes);
            let before = store.objects.lock().clone();
            assert!(matches!(open_from(store.clone(), 10), Err(error) if error == expected));
            assert_eq!(*store.objects.lock(), before);
            assert_eq!(store.operations.lock().len(), 2);
        }
    }

    #[test]
    fn complete_retired_pairs_reject_even_without_an_application_snapshot() {
        let ids = ObjectIds::new(10);
        for revision in [b'1', b'2'] {
            for retired in [vec![ids.state], vec![ids.log], vec![ids.state, ids.log]] {
                let store = current_pair();
                for id in retired {
                    let mut objects = store.objects.lock();
                    let record = objects.get_mut(&id).unwrap();
                    record[7] = revision;
                }
                let before = store.objects.lock().clone();
                assert!(matches!(
                    open_from(store.clone(), 10),
                    Err(StoreError::UnsupportedFormat(_))
                ));
                assert_eq!(*store.objects.lock(), before);
                assert_eq!(store.operations.lock().len(), 2);
            }
        }
    }

    #[test]
    fn term_vote_admission_snapshot_and_suffix_survive_reopen_and_explicit_reset() {
        let store = current_pair();
        let (log, state) = open_from(store.clone(), 10).unwrap();
        state.set_current_term(7);
        state.set_voted_for(Some(String::from("peer")));
        state.set_join_admission(Some(JoinAdmission {
            anchor_id: String::from("anchor"),
            snapshot_index: 4,
        }));
        log.append(vec![
            LogEntry::new(5, String::from("peer"), vec![1]),
            LogEntry::new(5, String::from("peer"), vec![2]),
            LogEntry::new(6, String::from("peer"), vec![3]),
        ]);
        log.install_snapshot(2, 5, vec![0x31, 0x32]);
        drop((log, state));
        let (log, state) = open_from(store.clone(), 10).unwrap();
        assert_eq!(state.current_term(), 7);
        assert_eq!(state.voted_for().as_deref(), Some("peer"));
        assert_eq!(state.join_admission().unwrap().snapshot_index, 4);
        assert_eq!(state.join_admission().unwrap().anchor_id, "anchor");
        assert_eq!(
            (log.snapshot_index(), log.snapshot_term(), log.last_index(), log.last_term()),
            (2, 5, 3, 6)
        );
        assert_eq!(log.snapshot_data(), [0x31, 0x32]);
        assert_eq!(log.entry_at(3).unwrap().data, [3]);
        log.install_snapshot(2, 5, vec![0x41]);
        assert_eq!(log.last_index(), 3);
        log.truncate_from(3);
        assert_eq!(open_from(store.clone(), 10).unwrap().0.last_index(), 2);
        log.reset();
        let (log, state) = open_from(store, 10).unwrap();
        assert_eq!((log.snapshot_index(), log.snapshot_term(), log.last_index()), (0, 0, 0));
        assert!(log.snapshot_data().is_empty());
        assert_eq!(state.current_term(), 7);
    }

    #[test]
    fn strict_codecs_reject_truncation_overflow_invalid_ids_and_partial_entry_suffixes() {
        let state = PersistentState {
            term: 7,
            voted_for: Some(String::from("vote")),
            admission: Some(JoinAdmission {
                anchor_id: String::from("anchor"),
                snapshot_index: 9,
            }),
        };
        let bytes = serialize_persistent_state(&state).unwrap();
        assert_eq!(decode_persistent_state(&bytes), Some(state));
        for end in 0..bytes.len() {
            assert!(decode_persistent_state(&bytes[..end]).is_none());
        }
        let mut invalid = bytes.clone();
        invalid[32] = 0xff;
        assert!(decode_persistent_state(&invalid).is_none());
        let mut invalid = bytes.clone();
        invalid.extend_from_slice(&[0]);
        assert!(decode_persistent_state(&invalid).is_none());
        let mut invalid = serialize_persistent_state(&PersistentState::default()).unwrap();
        invalid[24..32].copy_from_slice(&1u64.to_le_bytes());
        assert!(decode_persistent_state(&invalid).is_none());
        let state = LogState {
            snapshot_index: 3,
            snapshot_term: 5,
            snapshot: vec![1, 2],
            entries: vec![
                LogEntry::new(5, String::from("peer"), vec![3]),
                LogEntry::new(6, String::from("peer"), vec![4]),
            ],
        };
        let bytes = serialize_log_state(&state).unwrap();
        assert_eq!(decode_log_state(&bytes), Some(state));
        for end in 0..bytes.len() {
            assert!(decode_log_state(&bytes[..end]).is_none());
        }
        let mut invalid = bytes.clone();
        invalid[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode_log_state(&invalid).is_none());
        let mut invalid = bytes.clone();
        invalid[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode_log_state(&invalid).is_none());
        let mut invalid = bytes.clone();
        invalid[40 + 2 + 21 + 16] = 0xff;
        assert!(
            decode_log_state(&invalid).is_none(),
            "bad second entry must not return the valid first entry"
        );
        let mut invalid = bytes.clone();
        invalid.push(0);
        let len = invalid.len() as u64 - 42;
        invalid[32..40].copy_from_slice(&len.to_le_bytes());
        assert!(decode_log_state(&invalid).is_none(), "partial entry must not return a prefix");
        assert!(
            serialize_log_state(&LogState {
                entries: vec![LogEntry::new(1, "x".repeat(256), vec![])],
                ..LogState::default()
            })
            .is_none()
        );
        assert!(
            serialize_persistent_state(&PersistentState {
                voted_for: Some("x".repeat(257)),
                ..PersistentState::default()
            })
            .is_none()
        );
        assert!(
            serialize_persistent_state(&PersistentState {
                admission: Some(JoinAdmission {
                    anchor_id: String::new(),
                    snapshot_index: 0
                }),
                ..PersistentState::default()
            })
            .is_none()
        );
    }
}
