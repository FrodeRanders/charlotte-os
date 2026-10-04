//! Unified, per-address-space object-capability namespace.
//!
//! Public capability values are allocated here, rather than independently by
//! each kernel subsystem. Handles remain opaque; their authoritative table
//! entries carry the object-family tag.

use alloc::collections::BTreeMap;

use spin::LazyLock;

use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    memory::AddressSpaceId,
};

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

// Timer completions and thread-exit observers resolve capabilities from timer
// interrupt/tail context. This must therefore be the kernel's IRQ-safe mutex
// rather than `spin::Mutex`: a timer can otherwise preempt a table owner and
// leave an interrupt-context caller spinning for a thread that is no longer
// running. The IRQ-safe mutex masks local interrupts for the complete
// ownership interval and restores their prior state on unlock.
static CAPABILITIES: LazyLock<Mutex<BTreeMap<AddressSpaceId, AddressSpaceCapabilities>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// Mint a fresh object capability in `owner`'s namespace.
pub fn allocate(owner: AddressSpaceId, kind: ObjectKind) -> ObjectCapability {
    try_allocate(owner, kind).expect("capability id overflow")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AllocationError {
    IdentityExhausted,
}

/// Fallible identity minting for callers with an admission/rollback boundary.
/// This checks serial exhaustion, not aggregate capability or heap admission;
/// existing infallible families still need transactional migration.
pub(crate) fn try_allocate(
    owner: AddressSpaceId,
    kind: ObjectKind,
) -> Result<ObjectCapability, AllocationError> {
    let mut tables = CAPABILITIES.lock();
    let table = tables.entry(owner).or_insert_with(AddressSpaceCapabilities::new);
    let (serial, next) = charlotte_lifecycle::claim_generation(table.next_serial)
        .ok_or(AllocationError::IdentityExhausted)?;
    table.next_serial = next;
    let cap = serial;
    let previous = table.objects.insert(cap, kind);
    debug_assert!(previous.is_none());
    Ok(cap)
}

pub(crate) fn test_identity_exhaustion() {
    const OWNER: AddressSpaceId = 0x5e31;
    let cap = try_allocate(OWNER, ObjectKind::Mailbox).unwrap();
    CAPABILITIES.lock().get_mut(&OWNER).unwrap().next_serial = u64::MAX;
    assert_eq!(try_allocate(OWNER, ObjectKind::Mailbox), Err(AllocationError::IdentityExhausted));
    assert!(contains(OWNER, cap, ObjectKind::Mailbox));
    assert_eq!(CAPABILITIES.lock().get(&OWNER).unwrap().objects.len(), 1);
    assert!(remove(OWNER, cap, ObjectKind::Mailbox));
    close_address_space(OWNER);
}

/// Kernel-fixture injection into an otherwise isolated capability namespace.
/// The fixture must retire this namespace afterwards; identity is never reset.
pub(crate) fn exhaust_identity_for_test(owner: AddressSpaceId) {
    CAPABILITIES.lock().get_mut(&owner).unwrap().next_serial = u64::MAX;
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
