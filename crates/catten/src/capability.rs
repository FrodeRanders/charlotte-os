//! Unified, per-address-space object-capability namespace.
//!
//! Public capability values are allocated here, rather than independently by
//! each kernel subsystem. Handles remain opaque; their authoritative table
//! entries carry the object-family tag.

use alloc::collections::BTreeMap;

use spin::{
    LazyLock,
    Mutex,
};

use crate::memory::AddressSpaceId;

pub type ObjectCapability = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ObjectKind {
    Ipc = 1,
    Memory = 2,
    Completion = 3,
    Device = 4,
    Mailbox = 5,
    /// Authority to inspect system-wide, non-secret kernel telemetry.
    SystemObserver = 6,
}

#[derive(Debug)]
struct AddressSpaceCapabilities {
    next_serial: u64,
    objects: BTreeMap<ObjectCapability, ObjectKind>,
}

impl AddressSpaceCapabilities {
    fn new() -> Self {
        Self {
            next_serial: 1,
            objects: BTreeMap::new(),
        }
    }
}

static CAPABILITIES: LazyLock<Mutex<BTreeMap<AddressSpaceId, AddressSpaceCapabilities>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// Mint a fresh object capability in `owner`'s namespace.
pub fn allocate(owner: AddressSpaceId, kind: ObjectKind) -> ObjectCapability {
    let mut tables = CAPABILITIES.lock();
    let table = tables.entry(owner).or_insert_with(AddressSpaceCapabilities::new);
    let serial = table.next_serial;
    table.next_serial = serial.checked_add(1).expect("capability id overflow");
    let cap = serial;
    let previous = table.objects.insert(cap, kind);
    debug_assert!(previous.is_none());
    cap
}

/// Check both ownership and object kind.
pub fn contains(owner: AddressSpaceId, cap: ObjectCapability, kind: ObjectKind) -> bool {
    CAPABILITIES
        .lock()
        .get(&owner)
        .and_then(|table| table.objects.get(&cap))
        .is_some_and(|actual| *actual == kind)
}

/// Revoke a capability if it belongs to `owner` and has the expected kind.
pub fn remove(owner: AddressSpaceId, cap: ObjectCapability, kind: ObjectKind) -> bool {
    let mut tables = CAPABILITIES.lock();
    let Some(table) = tables.get_mut(&owner) else {
        return false;
    };
    if table.objects.get(&cap) != Some(&kind) {
        return false;
    }
    table.objects.remove(&cap);
    true
}

/// Restore the same authority during an internal transaction rollback.
///
/// This is deliberately crate-private: public delegation always mints a fresh
/// handle, while rollback must make the pre-transaction handle valid again.
pub(crate) fn restore(owner: AddressSpaceId, cap: ObjectCapability, kind: ObjectKind) -> bool {
    let mut tables = CAPABILITIES.lock();
    let table = tables.entry(owner).or_insert_with(AddressSpaceCapabilities::new);
    if table.objects.contains_key(&cap) {
        return false;
    }
    table.objects.insert(cap, kind);
    true
}

/// Drop the complete authority namespace after subsystem payload teardown.
pub fn close_address_space(owner: AddressSpaceId) {
    CAPABILITIES.lock().remove(&owner);
}

#[cfg(test)]
pub fn kind_of(owner: AddressSpaceId, cap: ObjectCapability) -> Option<ObjectKind> {
    CAPABILITIES.lock().get(&owner).and_then(|table| table.objects.get(&cap)).copied()
}
