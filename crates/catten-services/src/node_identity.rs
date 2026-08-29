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
use alloc::{
    format,
    vec,
    vec::Vec,
};

use catten_syscall::*;

const REPLY_SPINS: u64 = u64::MAX;
const BUFFER_VADDR: usize = 0x0000_0000_2000_0000;
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
    /// fresh identity.
    pub fn load_or_create(ns_conn: u64, mnemonic: &[u8], mac: Option<[u8; 6]>) -> Option<Self> {
        if mnemonic.is_empty() {
            return None;
        }
        let obj_conn = objstore_connect(ns_conn)?;
        let namespace = fnv1a(mnemonic);
        let object = stable_object_id(namespace);
        if let Some(bytes) = obj_read(obj_conn, object)
            && let Some(identity) = decode(&bytes)
        {
            return Some(identity);
        }

        let mac = mac?;
        let token = fnv1a(&mac);
        let name = format!(
            "{}:{:08x}",
            core::str::from_utf8(mnemonic).unwrap_or("node"),
            token & 0xffff_ffff
        )
        .into_bytes();

        let mut blob = vec![0u8; 16 + mnemonic.len() + name.len()];
        blob[0..8].copy_from_slice(&IDENTITY_MAGIC.to_le_bytes());
        blob[8..12].copy_from_slice(&(mnemonic.len() as u32).to_le_bytes());
        blob[12..12 + mnemonic.len()].copy_from_slice(mnemonic);
        let name_off = 12 + mnemonic.len();
        blob[name_off..name_off + 4].copy_from_slice(&(name.len() as u32).to_le_bytes());
        blob[name_off + 4..].copy_from_slice(&name);

        let _ = obj_create_at(obj_conn, object);
        if obj_write(obj_conn, object, &blob) && obj_flush(obj_conn) {
            Some(NodeIdentity {
                mnemonic: mnemonic.to_vec(),
                name,
            })
        } else {
            None
        }
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
    if bytes.len() < 16 {
        return None;
    }
    if u64::from_le_bytes(bytes[0..8].try_into().ok()?) != IDENTITY_MAGIC {
        return None;
    }
    let mnemonic_len = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
    let name_len_off = 12 + mnemonic_len;
    if name_len_off + 4 > bytes.len() {
        return None;
    }
    let name_len =
        u32::from_le_bytes(bytes[name_len_off..name_len_off + 4].try_into().ok()?) as usize;
    let name_off = name_len_off + 4;
    if name_off + name_len > bytes.len() || mnemonic_len == 0 || name_len == 0 {
        return None;
    }
    Some(NodeIdentity {
        mnemonic: bytes[12..name_len_off].to_vec(),
        name: bytes[name_off..name_off + name_len].to_vec(),
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

fn objstore_connect(ns_conn: u64) -> Option<u64> {
    // The identity lives on NVMe, so wait for the object store to register
    // (deferred lookup) rather than failing on first boot.
    let lookup = ipc_scalar_call_connection(
        ns_conn,
        crate::ns::OP_LOOKUP,
        crate::objstore::NAME,
        0,
        IpcRights::SEND | IpcRights::CALL,
    );
    if lookup == 0 {
        return None;
    }
    let (generation, conn) = unsafe { crate::wait_reply(lookup, u64::MAX) };
    if generation < 1 || conn == 0 {
        None
    } else {
        Some(conn)
    }
}

fn obj_create_at(obj_conn: u64, object_id: u64) -> bool {
    let call = ipc_scalar_call(obj_conn, charlotte_protocol_objstore::OP_CREATE_AT, object_id);
    if call == 0 {
        return false;
    }
    let (result, _) = unsafe { crate::wait_reply(call, REPLY_SPINS) };
    result == charlotte_protocol_objstore::ERR_OK
        || result == charlotte_protocol_objstore::ERR_EXISTS
}

fn obj_write(obj_conn: u64, object_id: u64, data: &[u8]) -> bool {
    let size_mem = memory_alloc(1);
    if size_mem == 0 || memory_map(size_mem, BUFFER_VADDR, true) != 0 {
        if size_mem != 0 {
            memory_close(size_mem);
        }
        return false;
    }
    unsafe {
        (BUFFER_VADDR as *mut u64).write_unaligned(data.len() as u64);
    }
    memory_unmap(size_mem);
    let size_call =
        ipc_scalar_call_borrow_read(obj_conn, crate::objstore::OP_SET_SIZE, object_id, size_mem);
    if size_call == 0 {
        memory_close(size_mem);
        return false;
    }
    let (size_result, _) = unsafe { crate::wait_reply(size_call, REPLY_SPINS) };
    memory_close(size_mem);
    if size_result != 0 {
        return false;
    }

    let pages = data.len().max(1).div_ceil(4096);
    let mem = memory_alloc(pages);
    if mem == 0 {
        return false;
    }
    if memory_map(mem, BUFFER_VADDR, true) != 0 {
        memory_close(mem);
        return false;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), BUFFER_VADDR as *mut u8, data.len());
    }
    memory_unmap(mem);
    let call = ipc_scalar_call_move(obj_conn, crate::objstore::OP_WRITE, object_id, mem);
    if call == 0 {
        return false;
    }
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
    if status != 0 || memory == 0 {
        if memory != 0 {
            memory_close(memory);
        }
        return None;
    }
    if memory_map(memory, BUFFER_VADDR, false) != 0 {
        memory_close(memory);
        return None;
    }
    let size = result as usize;
    let mut buf = vec![0u8; size];
    unsafe {
        core::ptr::copy_nonoverlapping(BUFFER_VADDR as *const u8, buf.as_mut_ptr(), size);
    }
    memory_unmap(memory);
    memory_close(memory);
    Some(buf)
}

fn obj_flush(obj_conn: u64) -> bool {
    let call = ipc_scalar_call_connection(
        obj_conn,
        crate::objstore::OP_FLUSH,
        0,
        0,
        IpcRights::SEND | IpcRights::CALL,
    );
    if call == 0 {
        return false;
    }
    let (result, _) = unsafe { crate::wait_reply(call, REPLY_SPINS) };
    result == 0
}
