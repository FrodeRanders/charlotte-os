//! Persistent, cluster-scoped node identity.
//!
//! A node's name is `{cluster_mnemonic}:{token}` where `token` is an
//! eight-hex-digit FNV-1a of the node's NIC MAC. The identity is derived once
//! on first boot and persisted to the NVMe-backed object store, so the node
//! keeps the same name across reboots even if its NIC changes. The cluster
//! mnemonic supplies the cluster context; the MAC-derived token guarantees
//! uniqueness, and the name becomes official only once ratified through the
//! replicated Raft membership.
//!
//! ## Persistence layout
//!
//! The identity lives in a namespace derived from the cluster mnemonic, at a
//! stable high object id:
//! ```text
//! 0..8    magic (u64 LE)
//! 8..12   mnemonic length (u32 LE)
//! 12..    mnemonic bytes
//! ..+4    name length (u32 LE)
//! ..      name bytes
//! ```
use alloc::vec::Vec;

use catten_rt::owned::{
    Connection,
    ConnectionRef,
    IpcError,
    OwnedMemory,
    PendingCall,
};

use crate::{
    deadline::Deadline,
    objstore,
    sleep_ms,
};

const PUBLICATION_WAIT_MS: u64 = 10_000;
const MAX_IDENTITY_BYTES: usize = 4096;

trait IdentityStore {
    fn read(&self, object: u64) -> Option<Option<Vec<u8>>>;
    fn create_shared(&self, object: u64) -> Option<bool>;
    fn write(&self, object: u64, bytes: &[u8]) -> Option<()>;
    fn flush(&self) -> Option<()>;
}

/// The connection and total admission deadline belong to this transaction.
/// Persistent object IDs name durable data; Drop leaves those objects intact.
struct Store {
    connection: Connection,
    deadline: Deadline,
}

impl Store {
    fn call(&self, opcode: u32, object: u64) -> Option<PendingCall<'static>> {
        loop {
            match self.connection.call(opcode, object) {
                Ok(call) => return Some(call),
                Err(IpcError::Status(catten_syscall::ipc_status::QUEUE_FULL))
                    if !self.deadline.expired() =>
                {
                    sleep_ms(10)
                }
                Err(_) => return None,
            }
        }
    }

    fn call_move(
        &self,
        opcode: u32,
        object: u64,
        mut memory: OwnedMemory,
    ) -> Option<PendingCall<'static>> {
        loop {
            match self.connection.call_move(opcode, object, memory) {
                Ok(call) => return Some(call),
                Err((returned, error)) => {
                    // The move was rejected before publication. Keep its exact
                    // returned owner; retry only exact queue-capacity rejection.
                    memory = returned;
                    if error != IpcError::Status(catten_syscall::ipc_status::QUEUE_FULL)
                        || self.deadline.expired()
                    {
                        return None;
                    }
                    sleep_ms(10);
                }
            }
        }
    }
}

impl IdentityStore for Store {
    fn read(&self, object: u64) -> Option<Option<Vec<u8>>> {
        let result = self.call(objstore::OP_READ, object)?.wait().ok()?;
        if result.result == objstore::ERR_NOT_FOUND && result.memory.is_none() {
            return Some(None);
        }
        let length = usize::try_from(result.result).ok()?;
        if length > MAX_IDENTITY_BYTES {
            return None;
        }
        let memory = result.memory?;
        if length > memory.len() {
            return None;
        }
        let mapping = memory.map_read_only().ok()?;
        Some(Some(copy_bytes(&mapping.as_slice()[..length])?))
    }

    fn create_shared(&self, object: u64) -> Option<bool> {
        match self.call(objstore::OP_CREATE_AT_SHARED, object)?.wait().ok()?.result {
            objstore::ERR_OK => Some(true),
            objstore::ERR_EXISTS => {
                catten_rt::logln!("[identity] shared creation lost; waiting for owner publication");
                Some(false)
            }
            _ => None,
        }
    }

    fn write(&self, object: u64, bytes: &[u8]) -> Option<()> {
        let size = OwnedMemory::allocate(1).ok()?;
        let mut mapping = size.map_writable().ok()?;
        mapping.as_mut_slice()[..8].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
        let size = mapping.unmap().ok()?;
        if self.call_move(objstore::OP_SET_SIZE, object, size)?.wait().ok()?.result
            != objstore::ERR_OK
        {
            return None;
        }
        let memory = OwnedMemory::allocate(bytes.len().max(1).div_ceil(4096)).ok()?;
        let mut mapping = memory.map_writable().ok()?;
        mapping.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
        let memory = mapping.unmap().ok()?;
        (self.call_move(objstore::OP_WRITE, object, memory)?.wait().ok()?.result
            == objstore::ERR_OK)
            .then_some(())
    }

    fn flush(&self) -> Option<()> {
        (self.call(objstore::OP_FLUSH, 0)?.wait().ok()?.result == objstore::ERR_OK).then_some(())
    }
}

fn copy_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut owned = Vec::new();
    owned.try_reserve_exact(bytes.len()).ok()?;
    owned.extend_from_slice(bytes);
    Some(owned)
}

fn encoded(mnemonic: &[u8], mac: [u8; 6]) -> Option<Vec<u8>> {
    core::str::from_utf8(mnemonic).ok()?;
    let name_len = mnemonic.len().checked_add(9)?;
    let length = 16usize.checked_add(mnemonic.len())?.checked_add(name_len)?;
    if length > MAX_IDENTITY_BYTES {
        return None;
    }
    let mut blob = Vec::new();
    blob.try_reserve_exact(length).ok()?;
    blob.resize(length, 0);
    blob[..8].copy_from_slice(&IDENTITY_MAGIC.to_le_bytes());
    blob[8..12].copy_from_slice(&u32::try_from(mnemonic.len()).ok()?.to_le_bytes());
    blob[12..12 + mnemonic.len()].copy_from_slice(mnemonic);
    let offset = 12 + mnemonic.len();
    blob[offset..offset + 4].copy_from_slice(&u32::try_from(name_len).ok()?.to_le_bytes());
    let name = &mut blob[offset + 4..];
    name[..mnemonic.len()].copy_from_slice(mnemonic);
    name[mnemonic.len()] = b':';
    let token = fnv1a(&mac) as u32;
    for digit in 0..8 {
        name[mnemonic.len() + 1 + digit] =
            b"0123456789abcdef"[((token >> (28 - digit * 4)) & 15) as usize];
    }
    Some(blob)
}

fn load_from(
    store: &impl IdentityStore,
    mnemonic: &[u8],
    mac: Option<[u8; 6]>,
    mut wait: impl FnMut() -> bool,
) -> Option<NodeIdentity> {
    if mnemonic.is_empty() {
        return None;
    }
    let object = stable_object_id(fnv1a(mnemonic));
    let initial = store.read(object)?;
    if let Some(bytes) = &initial
        && !bytes.is_empty()
        && let Some(identity) = decode(bytes)
    {
        return (identity.mnemonic == mnemonic).then_some(identity);
    }
    if initial.is_none() {
        let blob = encoded(mnemonic, mac?)?;
        if store.create_shared(object)? {
            // Only exact successful creation authorizes this writer. Existing
            // shared-read state is never permission to overwrite its owner.
            store.write(object, &blob)?;
            store.flush()?;
            return decode(&blob);
        }
    }
    while wait() {
        if let Some(bytes) = store.read(object)?
            && !bytes.is_empty()
        {
            // SET_SIZE can expose zero-filled storage before the creator's
            // atomic write. A creation loser waits for a complete encoding,
            // never repairs or overwrites the other owner's preparation.
            if let Some(identity) = decode(&bytes) {
                return (identity.mnemonic == mnemonic).then_some(identity);
            }
        }
    }
    None
}

const IDENTITY_MAGIC: u64 = 0x4e4f_4445_4944_524f; // "NODEIDRO"
/// Stable high-range object id for the identity blob (out of the low range
/// the object store hands out monotonically).
const IDENTITY_BASE: u64 = 0x9000_0000_0000_0000;

/// The node's stable, cluster-scoped identity.
pub struct NodeIdentity {
    /// User-chosen cluster identifier (for example, "charlotte").
    pub mnemonic: Vec<u8>,
    /// The node's name: `{mnemonic}:{token}`.
    pub name: Vec<u8>,
}

impl NodeIdentity {
    /// Load the persisted identity for `mnemonic`, or derive a fresh one from
    /// the NIC MAC and persist it.
    ///
    /// `mac` is required only on first boot, when no identity has been
    /// persisted yet. Returns `None` when the object store is unavailable, the
    /// persisted identity is malformed, or no MAC is available to derive a
    /// fresh identity. Only successful creation authorizes a write. A shared
    /// creation loser waits for complete owner publication within ten seconds
    /// of connecting; name lookup and an already accepted RPC are not bounded
    /// by that admission deadline. Existing corrupt state is never overwritten.
    pub fn load_or_create(
        ns_connection: ConnectionRef<'_>,
        mnemonic: &[u8],
        mac: Option<[u8; 6]>,
    ) -> Option<Self> {
        let (_, connection) = crate::wait_for_registered_name_owned(ns_connection, objstore::NAME)?;
        let store = Store {
            connection,
            deadline: Deadline::after(PUBLICATION_WAIT_MS),
        };
        load_from(&store, mnemonic, mac, || {
            if store.deadline.expired() {
                false
            } else {
                sleep_ms(10);
                true
            }
        })
    }

    /// The node's name as a string slice.
    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name).unwrap_or("")
    }
}

fn stable_object_id(namespace: u64) -> u64 {
    // Distinct high base from the Raft stores (0x8000...), so a node identity
    // can never alias a Raft log/persistent-state object. Per-node disk means
    // this only needs to be unique within one node's object store.
    IDENTITY_BASE | (namespace & 0x0fff_ffff_ffff_ffff)
}

fn decode(bytes: &[u8]) -> Option<NodeIdentity> {
    if !(16..=MAX_IDENTITY_BYTES).contains(&bytes.len()) {
        return None;
    }
    if u64::from_le_bytes(bytes[0..8].try_into().ok()?) != IDENTITY_MAGIC {
        return None;
    }
    let mnemonic_len = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
    let name_len_off = 12usize.checked_add(mnemonic_len)?;
    if name_len_off.checked_add(4)? > bytes.len() {
        return None;
    }
    let name_len =
        u32::from_le_bytes(bytes[name_len_off..name_len_off + 4].try_into().ok()?) as usize;
    let name_off = name_len_off + 4;
    if name_off.checked_add(name_len)? != bytes.len() || mnemonic_len == 0 || name_len == 0 {
        return None;
    }
    let mnemonic = &bytes[12..name_len_off];
    let name = &bytes[name_off..];
    if name.len() != mnemonic.len().checked_add(9)?
        || !name.starts_with(mnemonic)
        || name.get(mnemonic.len()) != Some(&b':')
        || key_from_name(name).is_none()
    {
        return None;
    }
    Some(NodeIdentity {
        mnemonic: copy_bytes(&bytes[12..name_len_off])?,
        name: copy_bytes(&bytes[name_off..name_off + name_len])?,
    })
}

/// FNV-1a 64-bit (offset basis + prime), the same helper the Raft stores use
/// to derive stable object namespaces.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Recover the stable 32-bit node key from a ratified
/// `{cluster-mnemonic}:{eight-hex-digit-key}` node name.
///
/// Deployment records use this compact key while the replicated name catalog
/// retains the human-readable node identity. Keeping the conversion here
/// prevents rollout observers from depending on the mnemonic.
pub fn key_from_name(name: &[u8]) -> Option<u64> {
    let separator = name.iter().rposition(|byte| *byte == b':')?;
    let token = name.get(separator + 1..)?;
    if token.len() != 8 {
        return None;
    }
    token.iter().try_fold(0u64, |value, byte| {
        let digit = match byte {
            b'0'..=b'9' => u64::from(byte - b'0'),
            b'a'..=b'f' => u64::from(byte - b'a') + 10,
            b'A'..=b'F' => u64::from(byte - b'A') + 10,
            _ => return None,
        };
        value.checked_mul(16)?.checked_add(digit)
    })
}

#[cfg(test)]
mod tests {
    use alloc::{
        collections::VecDeque,
        vec,
    };
    use core::cell::{
        Cell,
        RefCell,
    };

    use super::*;

    struct ScriptedStore {
        reads: RefCell<VecDeque<Option<Vec<u8>>>>,
        created: bool,
        write_ok: bool,
        creates: Cell<usize>,
        writes: Cell<usize>,
        flushes: Cell<usize>,
    }

    impl ScriptedStore {
        fn new(reads: Vec<Option<Vec<u8>>>, created: bool) -> Self {
            Self {
                reads: RefCell::new(reads.into()),
                created,
                write_ok: true,
                creates: Cell::new(0),
                writes: Cell::new(0),
                flushes: Cell::new(0),
            }
        }
    }
    impl IdentityStore for ScriptedStore {
        fn read(&self, _: u64) -> Option<Option<Vec<u8>>> {
            self.reads.borrow_mut().pop_front()
        }

        fn create_shared(&self, _: u64) -> Option<bool> {
            self.creates.set(self.creates.get() + 1);
            Some(self.created)
        }

        fn write(&self, _: u64, bytes: &[u8]) -> Option<()> {
            assert!(self.created, "foreign creation is never a write grant");
            assert!(decode(bytes).is_some());
            self.writes.set(self.writes.get() + 1);
            self.write_ok.then_some(())
        }

        fn flush(&self) -> Option<()> {
            self.flushes.set(self.flushes.get() + 1);
            Some(())
        }
    }

    #[test]
    fn creation_loser_waits_for_canonical_owner_without_writing() {
        let published = encoded(b"charlotte", [1; 6]).unwrap();
        let store = ScriptedStore::new(
            vec![None, Some(Vec::new()), Some(vec![0; published.len()]), Some(published.clone())],
            false,
        );
        let mut waits = 0;
        let identity = load_from(&store, b"charlotte", Some([2; 6]), || {
            waits += 1;
            waits <= 3
        })
        .unwrap();
        assert_eq!(identity.name, decode(&published).unwrap().name);
        assert_eq!((store.creates.get(), store.writes.get(), store.flushes.get()), (1, 0, 0));
        assert_eq!(waits, 3);
        let store = ScriptedStore::new(
            vec![Some(vec![0; published.len()]), Some(published.clone())],
            false,
        );
        assert_eq!(
            load_from(&store, b"charlotte", None, || true).unwrap().name,
            decode(&published).unwrap().name
        );
        assert_eq!((store.creates.get(), store.writes.get(), store.flushes.get()), (0, 0, 0));
    }

    #[test]
    fn only_successful_creation_writes_and_flushes_once() {
        let store = ScriptedStore::new(vec![None], true);
        let identity =
            load_from(&store, b"charlotte", Some([3; 6]), || panic!("creator must not wait"))
                .unwrap();
        assert_eq!(identity.name, decode(&encoded(b"charlotte", [3; 6]).unwrap()).unwrap().name);
        assert_eq!((store.creates.get(), store.writes.get(), store.flushes.get()), (1, 1, 1));
        let mut store = ScriptedStore::new(vec![None], true);
        store.write_ok = false;
        assert!(
            load_from(&store, b"charlotte", Some([3; 6]), || panic!(
                "failed published writer must not retry"
            ))
            .is_none()
        );
        assert_eq!((store.creates.get(), store.writes.get(), store.flushes.get()), (1, 1, 0));
    }

    #[test]
    fn corrupt_foreign_and_unfinished_publication_are_not_overwritten() {
        let mut corrupt = encoded(b"charlotte", [1; 6]).unwrap();
        corrupt[0] ^= 1;
        for initial in [corrupt, encoded(b"other", [1; 6]).unwrap(), Vec::new()] {
            let store = ScriptedStore::new(vec![Some(initial)], false);
            assert!(load_from(&store, b"charlotte", Some([2; 6]), || false).is_none());
            assert_eq!((store.creates.get(), store.writes.get(), store.flushes.get()), (0, 0, 0));
        }
        let store = ScriptedStore::new(vec![None], false);
        assert!(load_from(&store, b"charlotte", None, || false).is_none());
        assert_eq!(store.creates.get(), 0);
    }
}
