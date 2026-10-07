//! Unified, per-address-space object-capability namespace.
//!
//! Public capability values are allocated here, rather than independently by
//! each kernel subsystem. Handles remain opaque; their authoritative table
//! entries carry the object-family tag.

use alloc::{
    collections::BTreeMap,
    sync::Arc,
};

use spin::LazyLock;

use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    memory::AddressSpaceId,
};

pub(crate) mod admission_tests;
mod budget;

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
    address_space: Option<crate::memory::AddressSpaceHandle>,
    platform: bool,
    budget: Arc<budget::DomainBudget>,
    objects: BTreeMap<ObjectCapability, Entry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryState {
    Staged,
    Live,
    Escrow,
}

#[derive(Debug)]
struct Entry {
    kind: ObjectKind,
    state: EntryState,
    _charge: budget::Charge,
}

impl AddressSpaceCapabilities {
    fn try_new(
        address_space: Option<crate::memory::AddressSpaceHandle>,
    ) -> Result<Self, AllocationError> {
        Ok(Self {
            next_serial: 1,
            address_space,
            platform: false,
            budget: budget::DomainBudget::try_new()?,
            objects: BTreeMap::new(),
        })
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

/// Prepare namespace metadata before allocating an ASID. Its publication is
/// infallible except for the kernel allocator used by BTreeMap itself. Empty
/// namespace metadata is not a charged capability record.
pub(crate) struct PreparingNamespace(AddressSpaceCapabilities);

pub(crate) fn prepare_namespace() -> Result<PreparingNamespace, AllocationError> {
    Ok(PreparingNamespace(AddressSpaceCapabilities::try_new(None)?))
}

impl PreparingNamespace {
    pub(crate) fn publish(mut self, handle: crate::memory::AddressSpaceHandle) {
        self.0.address_space = Some(handle);
        let previous = CAPABILITIES.lock().insert(handle.id(), self.0);
        assert!(previous.is_none(), "capability namespace survived address-space teardown");
    }
}

/// Only the existing kernel platform-launch path may promote future entries.
/// Old entries keep their original charge class until they are released.
pub(crate) fn mark_platform(handle: crate::memory::AddressSpaceHandle) {
    let mut tables = CAPABILITIES.lock();
    if let Some(table) = tables.get_mut(&handle.id())
        && table.address_space == Some(handle)
        && table.budget.accepting()
    {
        table.platform = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AllocationError {
    IdentityExhausted,
    ResourceLimit,
    Retired,
    AllocationFailed,
    UnknownCapability,
}

/// Fallible shared admission and identity minting. Multi-step callers reserve
/// a hidden identity first, then publish only when payload state is prepared.
pub(crate) fn try_allocate(
    owner: AddressSpaceId,
    kind: ObjectKind,
) -> Result<ObjectCapability, AllocationError> {
    reserve(owner, kind)?.publish()
}

// Borrow the actual lifecycle guard, not a boolean assertion from a caller.
// The crate-private helper's caller must own ADDRESS_SPACE_LIFECYCLE itself.
pub(crate) type LifecycleGuard<'a> =
    lock_api::MutexGuard<'a, crate::cpu::multiprocessor::spin::mutex::MutexCore, ()>;

fn namespace(
    tables: &mut BTreeMap<AddressSpaceId, AddressSpaceCapabilities>,
    owner: AddressSpaceId,
    identity: Option<crate::memory::AddressSpaceHandle>,
) -> Result<&mut AddressSpaceCapabilities, AllocationError> {
    Ok(match tables.entry(owner) {
        alloc::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
        alloc::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(AddressSpaceCapabilities::try_new(identity)?)
        }
    })
}

fn insert_entry(
    table: &mut AddressSpaceCapabilities,
    kind: ObjectKind,
    platform: bool,
    state: EntryState,
) -> Result<ObjectCapability, AllocationError> {
    let charge = budget::reserve(&table.budget, platform)?;
    let (serial, next) = charlotte_lifecycle::claim_generation(table.next_serial)
        .ok_or(AllocationError::IdentityExhausted)?;
    table.next_serial = next;
    let cap = serial;
    let previous = table.objects.insert(
        cap,
        Entry {
            kind,
            state,
            _charge: charge,
        },
    );
    debug_assert!(previous.is_none());
    Ok(cap)
}

/// Own a hidden identity and its shared count until publication or Drop. No
/// subsystem payload is owned here; that subsystem stages its own resources.
#[must_use]
#[derive(Debug)]
pub(crate) struct Reservation {
    owner: AddressSpaceId,
    cap: ObjectCapability,
    kind: ObjectKind,
    namespace: Arc<budget::DomainBudget>,
    active: bool,
}

pub(crate) fn reserve(
    owner: AddressSpaceId,
    kind: ObjectKind,
) -> Result<Reservation, AllocationError> {
    let lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
    reserve_in_lifecycle(owner, kind, &lifecycle)
}

pub(crate) fn try_allocate_in_lifecycle(
    owner: AddressSpaceId,
    kind: ObjectKind,
    lifecycle: &LifecycleGuard<'_>,
) -> Result<ObjectCapability, AllocationError> {
    reserve_in_lifecycle(owner, kind, lifecycle)?.publish()
}

pub(crate) fn reserve_in_lifecycle(
    owner: AddressSpaceId,
    kind: ObjectKind,
    _lifecycle: &LifecycleGuard<'_>,
) -> Result<Reservation, AllocationError> {
    // No address-space or memory-ledger lookup while CAPABILITIES is owned.
    // Lifecycle ownership prevents an absent namespace being recreated during
    // retirement. The resulting token does not retain that global guard.
    // The permanently reserved kernel namespace is not a reusable user ASID.
    let identity = if owner == crate::memory::KERNEL_ASID {
        None
    } else {
        crate::memory::current_address_space_handle(owner)
    };
    if identity.is_some_and(|handle| !crate::memory::budget::accepting(handle)) {
        return Err(AllocationError::Retired);
    }
    reserve_captured(owner, kind, identity)
}

/// Subsystem-serialized admission. The caller retains its registry guard and
/// supplies that registry's captured generation, not a fresh numeric lookup.
/// Real user namespaces are prepared at domain creation, never recreated here.
/// None is restricted to the permanent kernel and kernel-only pseudo domains.
pub(crate) fn reserve_captured(
    owner: AddressSpaceId,
    kind: ObjectKind,
    mut identity: Option<crate::memory::AddressSpaceHandle>,
) -> Result<Reservation, AllocationError> {
    if owner == crate::memory::KERNEL_ASID {
        identity = None;
    }
    let mut tables = CAPABILITIES.lock();
    if identity.is_some() && !tables.contains_key(&owner) {
        return Err(AllocationError::Retired);
    }
    let table = namespace(&mut tables, owner, identity)?;
    if table.address_space != identity {
        return Err(AllocationError::Retired);
    }
    let cap = insert_entry(
        table,
        kind,
        owner == crate::memory::KERNEL_ASID || table.platform,
        EntryState::Staged,
    )?;
    Ok(Reservation {
        owner,
        cap,
        kind,
        namespace: table.budget.clone(),
        active: true,
    })
}

impl Reservation {
    pub(crate) fn identity(&self) -> ObjectCapability {
        self.cap
    }

    pub(crate) fn publish(mut self) -> Result<ObjectCapability, AllocationError> {
        let mut tables = CAPABILITIES.lock();
        let table = tables.get_mut(&self.owner).ok_or(AllocationError::Retired)?;
        if !Arc::ptr_eq(&table.budget, &self.namespace) || !table.budget.accepting() {
            return Err(AllocationError::Retired);
        }
        let entry = table.objects.get_mut(&self.cap).ok_or(AllocationError::UnknownCapability)?;
        if entry.kind != self.kind || entry.state != EntryState::Staged {
            return Err(AllocationError::UnknownCapability);
        }
        entry.state = EntryState::Live;
        self.active = false;
        Ok(self.cap)
    }
}

fn discard_captured(
    owner: AddressSpaceId,
    cap: ObjectCapability,
    namespace: &Arc<budget::DomainBudget>,
    state: EntryState,
) {
    let mut tables = CAPABILITIES.lock();
    if let Some(table) = tables.get_mut(&owner)
        && Arc::ptr_eq(&table.budget, namespace)
        && table.objects.get(&cap).is_some_and(|entry| entry.state == state)
    {
        table.objects.remove(&cap);
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.active {
            discard_captured(self.owner, self.cap, &self.namespace, EntryState::Staged);
        }
    }
}

/// Keep a source slot charged but non-authoritative during a move transaction.
/// Drop commits revocation; restore consumes the token and revives only the
/// same live namespace. Payload rollback is the caller's separate obligation.
#[must_use]
#[derive(Debug)]
pub(crate) struct SourceEscrow {
    owner: AddressSpaceId,
    cap: ObjectCapability,
    kind: ObjectKind,
    namespace: Arc<budget::DomainBudget>,
    active: bool,
}

pub(crate) fn escrow(
    owner: AddressSpaceId,
    cap: ObjectCapability,
    kind: ObjectKind,
) -> Result<SourceEscrow, AllocationError> {
    let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
    let identity = crate::memory::current_address_space_handle(owner);
    escrow_captured(owner, cap, kind, identity)
}

/// Caller owns the payload registry and captures generation before entering
/// it. Do not acquire lifecycle under that registry.
pub(crate) fn escrow_captured(
    owner: AddressSpaceId,
    cap: ObjectCapability,
    kind: ObjectKind,
    mut identity: Option<crate::memory::AddressSpaceHandle>,
) -> Result<SourceEscrow, AllocationError> {
    if owner == crate::memory::KERNEL_ASID {
        identity = None;
    }
    let mut tables = CAPABILITIES.lock();
    let table = tables.get_mut(&owner).ok_or(AllocationError::UnknownCapability)?;
    if table.address_space != identity || !table.budget.accepting() {
        return Err(AllocationError::Retired);
    }
    let entry = table.objects.get_mut(&cap).ok_or(AllocationError::UnknownCapability)?;
    if entry.kind != kind || entry.state != EntryState::Live {
        return Err(AllocationError::UnknownCapability);
    }
    entry.state = EntryState::Escrow;
    Ok(SourceEscrow {
        owner,
        cap,
        kind,
        namespace: table.budget.clone(),
        active: true,
    })
}

impl SourceEscrow {
    pub(crate) fn restore(self) -> Result<ObjectCapability, AllocationError> {
        self.restore_inner(false)
    }

    /// Cancel back to existing payload authority for teardown too. No fresh
    /// charge or replacement namespace may be used. Retirement may still need
    /// this original handle to drain the payload; it does not admit new work.
    pub(crate) fn rollback(self) -> Result<ObjectCapability, AllocationError> {
        self.restore_inner(true)
    }

    fn restore_inner(mut self, allow_retired: bool) -> Result<ObjectCapability, AllocationError> {
        let mut tables = CAPABILITIES.lock();
        let table = tables.get_mut(&self.owner).ok_or(AllocationError::Retired)?;
        if !Arc::ptr_eq(&table.budget, &self.namespace)
            || (!allow_retired && !table.budget.accepting())
        {
            return Err(AllocationError::Retired);
        }
        let entry = table.objects.get_mut(&self.cap).ok_or(AllocationError::UnknownCapability)?;
        if entry.kind != self.kind || entry.state != EntryState::Escrow {
            return Err(AllocationError::UnknownCapability);
        }
        entry.state = EntryState::Live;
        self.active = false;
        Ok(self.cap)
    }
}

impl Drop for SourceEscrow {
    fn drop(&mut self) {
        if self.active {
            discard_captured(self.owner, self.cap, &self.namespace, EntryState::Escrow);
        }
    }
}

/// Whether successful publication transfers or only lends source authority.
pub(crate) enum SourceDisposition {
    Revoke,
    Restore,
}

pub(crate) struct Publication<'a> {
    pub destination: &'a mut Reservation,
    pub source: Option<(&'a mut SourceEscrow, SourceDisposition)>,
}

/// Validate the entire mixed batch before changing any authority. Caller owns
/// the payload registry through publication and its remaining payload updates.
pub(crate) fn publish_batch(batch: &mut [Publication<'_>]) -> Result<(), AllocationError> {
    let mut tables = CAPABILITIES.lock();
    for publication in batch.iter() {
        let destination = &publication.destination;
        let mut entries = [
            Some((
                destination.owner,
                destination.cap,
                destination.kind,
                &destination.namespace,
                EntryState::Staged,
            )),
            publication.source.as_ref().map(|(source, _)| {
                (source.owner, source.cap, source.kind, &source.namespace, EntryState::Escrow)
            }),
        ];
        for (owner, cap, kind, namespace, state) in entries.iter_mut().filter_map(Option::take) {
            let table = tables.get(&owner).ok_or(AllocationError::Retired)?;
            if !Arc::ptr_eq(&table.budget, namespace) || !table.budget.accepting() {
                return Err(AllocationError::Retired);
            }
            if !table
                .objects
                .get(&cap)
                .is_some_and(|entry| entry.kind == kind && entry.state == state)
            {
                return Err(AllocationError::UnknownCapability);
            }
        }
    }
    for publication in batch.iter_mut() {
        let destination = &mut publication.destination;
        tables
            .get_mut(&destination.owner)
            .unwrap()
            .objects
            .get_mut(&destination.cap)
            .unwrap()
            .state = EntryState::Live;
        if let Some((source, disposition)) = &mut publication.source {
            let table = tables.get_mut(&source.owner).unwrap();
            match disposition {
                SourceDisposition::Revoke => {
                    table.objects.remove(&source.cap);
                }
                SourceDisposition::Restore => {
                    table.objects.get_mut(&source.cap).unwrap().state = EntryState::Live;
                }
            }
            source.active = false;
        }
        destination.active = false;
    }
    Ok(())
}

pub(crate) fn test_identity_exhaustion() {
    const OWNER: AddressSpaceId = 0x5e31;
    let cap = try_allocate(OWNER, ObjectKind::Mailbox).unwrap();
    CAPABILITIES.lock().get_mut(&OWNER).unwrap().next_serial = u64::MAX;
    assert_eq!(try_allocate(OWNER, ObjectKind::Mailbox), Err(AllocationError::IdentityExhausted));
    assert!(contains(OWNER, cap, ObjectKind::Mailbox));
    assert_eq!(CAPABILITIES.lock().get(&OWNER).unwrap().objects.len(), 1);
    assert_eq!(CAPABILITIES.lock().get(&OWNER).unwrap().budget.used(), 1);
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
        .is_some_and(|entry| entry.kind == kind && entry.state == EntryState::Live)
}

/// Diagnostic counts include staged reservations as well as live authority.
pub(crate) fn node_admission_used() -> (usize, usize) {
    budget::node_used()
}

/// Revoke a capability if it belongs to `owner` and has the expected kind.
pub fn remove(owner: AddressSpaceId, cap: ObjectCapability, kind: ObjectKind) -> bool {
    let mut tables = CAPABILITIES.lock();
    let Some(table) = tables.get_mut(&owner) else {
        return false;
    };
    if !table
        .objects
        .get(&cap)
        .is_some_and(|entry| entry.kind == kind && entry.state == EntryState::Live)
    {
        return false;
    }
    table.objects.remove(&cap);
    true
}

/// Trusted payload teardown can also revoke an escrowed source. Its retained
/// backing pin outlives the entry, and late cancellation cannot revive it.
pub(crate) fn remove_for_teardown(
    owner: AddressSpaceId,
    cap: ObjectCapability,
    kind: ObjectKind,
) -> bool {
    let mut tables = CAPABILITIES.lock();
    let Some(table) = tables.get_mut(&owner) else {
        return false;
    };
    if !table
        .objects
        .get(&cap)
        .is_some_and(|entry| entry.kind == kind && entry.state != EntryState::Staged)
    {
        return false;
    }
    table.objects.remove(&cap);
    true
}

/// Drop the complete authority namespace after subsystem payload teardown.
pub fn close_address_space(owner: AddressSpaceId) {
    if let Some(table) = CAPABILITIES.lock().remove(&owner) {
        table.budget.retire();
    }
}

/// Fence bounded reservations/publication before subsystem payload teardown.
pub(crate) fn retire_address_space(owner: AddressSpaceId) {
    if let Some(table) = CAPABILITIES.lock().get(&owner) {
        table.budget.retire();
    }
}

#[cfg(test)]
pub fn kind_of(owner: AddressSpaceId, cap: ObjectCapability) -> Option<ObjectKind> {
    CAPABILITIES
        .lock()
        .get(&owner)
        .and_then(|table| table.objects.get(&cap))
        .map(|entry| entry.kind)
}
