//! Unified, per-address-space object-capability namespace.
//!
//! Public capability values are allocated here, rather than independently by
//! each kernel subsystem. Handles remain opaque; their authoritative table
//! entries carry the object-family tag.

use alloc::sync::Arc;

use spin::LazyLock;

use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    klib::collections::retirement_list::{
        AdmittedMap,
        PreparedEntry,
    },
    memory::AddressSpaceId,
};

pub(crate) mod admission_tests;
mod budget;
mod namespace_tests;
mod record;
pub(crate) mod record_tests;
use record::PreparingRecord;
pub(crate) use record::RetiredRecord;

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
    objects: AdmittedMap<ObjectCapability, Entry>,
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
            objects: AdmittedMap::new(),
        })
    }
}

// Timer completions and thread-exit observers resolve capabilities from timer
// interrupt/tail context. This must therefore be the kernel's IRQ-safe mutex
// rather than `spin::Mutex`: a timer can otherwise preempt a table owner and
// leave an interrupt-context caller spinning for a thread that is no longer
// running. The IRQ-safe mutex masks local interrupts for the complete
// ownership interval and restores their prior state on unlock.
static CAPABILITIES: LazyLock<Mutex<AdmittedMap<AddressSpaceId, AddressSpaceCapabilities>>> =
    LazyLock::new(|| Mutex::new(AdmittedMap::new()));

/// Prepare the complete namespace and registry node before ASID publication.
/// Empty namespaces do not charge a capability record. Abandonment retains
/// both allocations without invoking their destructors beneath unknown guards.
#[must_use]
pub(crate) struct PreparingNamespace(core::mem::ManuallyDrop<NamespaceStorage>);
struct NamespaceStorage {
    node: Option<PreparedEntry<(AddressSpaceId, AddressSpaceCapabilities)>>,
    value: Option<AddressSpaceCapabilities>,
}

pub(crate) fn prepare_namespace() -> Result<PreparingNamespace, AllocationError> {
    if namespace_tests::reject(1) {
        return Err(AllocationError::AllocationFailed);
    }
    let node = PreparedEntry::try_new().map_err(|_| AllocationError::AllocationFailed)?;
    namespace_tests::boundary(false);
    let value = if namespace_tests::reject(2) {
        Err(AllocationError::AllocationFailed)
    } else {
        AddressSpaceCapabilities::try_new(None)
    };
    match value {
        Ok(value) => Ok(PreparingNamespace(core::mem::ManuallyDrop::new(NamespaceStorage {
            node: Some(node),
            value: Some(value),
        }))),
        Err(error) => {
            namespace_tests::boundary(true);
            drop(node);
            Err(error)
        }
    }
}

impl PreparingNamespace {
    pub(crate) fn publish(self, handle: crate::memory::AddressSpaceHandle) {
        self.publish_into(&mut CAPABILITIES.lock(), handle.id(), Some(handle));
    }

    fn publish_into(
        mut self,
        tables: &mut AdmittedMap<AddressSpaceId, AddressSpaceCapabilities>,
        owner: AddressSpaceId,
        identity: Option<crate::memory::AddressSpaceHandle>,
    ) {
        assert!(
            !tables.contains_key(&owner),
            "capability namespace survived address-space teardown"
        );
        let mut value = self.0.value.take().unwrap();
        value.address_space = identity;
        tables.insert(self.0.node.take().unwrap(), owner, value);
    }

    /// Ordinary unused preparation must leave local lifecycle/table/registry
    /// guards first. Drop deliberately cannot assume that context.
    pub(crate) fn cancel_unpublished(self) {
        namespace_tests::boundary(true);
        drop(core::mem::ManuallyDrop::into_inner(self.0));
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

fn namespace<'a>(
    tables: &'a mut AdmittedMap<AddressSpaceId, AddressSpaceCapabilities>,
    owner: AddressSpaceId,
    identity: Option<crate::memory::AddressSpaceHandle>,
    prepared: &mut Option<PreparingNamespace>,
) -> Result<&'a mut AddressSpaceCapabilities, AllocationError> {
    if !tables.contains_key(&owner) {
        // Real user roots prepare at registration. Only permanent kernel/raw
        // fixture namespaces can be created on this legacy caller boundary.
        if identity.is_some() {
            return Err(AllocationError::Retired);
        }
        prepared.take().ok_or(AllocationError::Retired)?.publish_into(tables, owner, None);
    }
    Ok(tables.get_mut(&owner).unwrap())
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
    identity: Option<crate::memory::AddressSpaceHandle>,
) -> Result<Reservation, AllocationError> {
    // Legacy captured callers may still hold outer guards. The same owner is
    // available to split-phase adapters that prepare before those guards.
    let mut preparation = PreparedReservation::try_new(owner, kind, identity)?;
    let result = preparation.reserve();
    preparation.finish();
    result
}

struct ReservationStorage {
    owner: AddressSpaceId,
    kind: ObjectKind,
    identity: Option<crate::memory::AddressSpaceHandle>,
    namespace: Option<PreparingNamespace>,
    record: PreparingRecord,
}
/// Own every fallible authority allocation before local publication guards.
/// Unused storage/charge must finish explicitly after those guards leave;
/// abandonment retains all fields without allocation, locks or destruction.
#[must_use]
pub(crate) struct PreparedReservation(core::mem::ManuallyDrop<ReservationStorage>);
impl PreparedReservation {
    pub(crate) fn try_new(
        owner: AddressSpaceId,
        kind: ObjectKind,
        mut identity: Option<crate::memory::AddressSpaceHandle>,
    ) -> Result<Self, AllocationError> {
        if owner == crate::memory::KERNEL_ASID {
            identity = None;
        }
        let record = PreparingRecord::try_new()?;
        let missing = identity.is_none() && !CAPABILITIES.lock().contains_key(&owner);
        let namespace = if missing {
            match prepare_namespace() {
                Ok(namespace) => Some(namespace),
                Err(error) => {
                    record.finish();
                    return Err(error);
                }
            }
        } else {
            None
        };
        Ok(Self(core::mem::ManuallyDrop::new(ReservationStorage {
            owner,
            kind,
            identity,
            namespace,
            record,
        })))
    }

    fn reserve(&mut self) -> Result<Reservation, AllocationError> {
        let storage = &mut *self.0;
        reserve_prepared_captured(
            storage.owner,
            storage.kind,
            storage.identity,
            &mut storage.namespace,
            &mut storage.record,
        )
    }

    /// Caller retains lifecycle and revalidates its captured root/closing fence.
    /// This path cannot allocate, dispose storage or acquire lifecycle itself.
    pub(crate) fn reserve_in_lifecycle(
        &mut self,
        _lifecycle: &LifecycleGuard<'_>,
    ) -> Result<Reservation, AllocationError> {
        if self.0.identity.is_some_and(|handle| !crate::memory::budget::accepting(handle)) {
            return Err(AllocationError::Retired);
        }
        self.reserve()
    }

    pub(crate) fn finish(self) {
        let storage = core::mem::ManuallyDrop::into_inner(self.0);
        if let Some(unused) = storage.namespace {
            unused.cancel_unpublished();
        }
        storage.record.finish();
    }
}

/// Same captured admission, with storage already owned by the preparation.
fn reserve_prepared_captured(
    owner: AddressSpaceId,
    kind: ObjectKind,
    identity: Option<crate::memory::AddressSpaceHandle>,
    prepared: &mut Option<PreparingNamespace>,
    record: &mut PreparingRecord,
) -> Result<Reservation, AllocationError> {
    let mut tables = CAPABILITIES.lock();
    if identity.is_some() && !tables.contains_key(&owner) {
        return Err(AllocationError::Retired);
    }
    let table = namespace(&mut tables, owner, identity, prepared)?;
    if table.address_space != identity {
        return Err(AllocationError::Retired);
    }
    let cap = record.insert(
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
) -> Option<RetiredRecord> {
    let mut tables = CAPABILITIES.lock();
    if let Some(table) = tables.get_mut(&owner)
        && Arc::ptr_eq(&table.budget, namespace)
        && table.objects.get(&cap).is_some_and(|entry| entry.state == state)
    {
        return table.objects.take(&cap).map(RetiredRecord::new);
    }
    None
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.active
            && let Some(retired) =
                discard_captured(self.owner, self.cap, &self.namespace, EntryState::Staged)
        {
            retired.release();
        }
    }
}

/// Keep a source slot charged but non-authoritative during a move transaction.
/// Drop commits revocation; restore consumes the token and revives only the
/// same live namespace. Payload rollback is the caller's separate obligation.
#[must_use]
#[derive(Debug)]
pub(crate) struct SourceEscrow {
    retired: Option<RetiredRecord>,
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
        retired: None,
        owner,
        cap,
        kind,
        namespace: table.budget.clone(),
        active: true,
    })
}

impl SourceEscrow {
    /// The containing transaction has finished payload publication and left
    /// its memory guard. A completed source node must not die inside the batch.
    pub(crate) fn finish_retired(&mut self) {
        if let Some(retired) = self.retired.take() {
            retired.release();
        }
    }

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
        if self.active
            && let Some(retired) =
                discard_captured(self.owner, self.cap, &self.namespace, EntryState::Escrow)
        {
            retired.release();
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
                    assert!(source.retired.is_none(), "source retirement owner replaced");
                    source.retired =
                        Some(RetiredRecord::new(table.objects.take(&source.cap).unwrap()));
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
    assert_eq!(CAPABILITIES.lock().get(&OWNER).unwrap().objects.iter().count(), 1);
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
    remove_with_state(owner, cap, kind, |state| state == EntryState::Live)
}

/// Detach only live authority into its original charged metadata owner. The
/// caller must retain payload/root dependencies and release after local guards.
pub(crate) fn detach(
    owner: AddressSpaceId,
    cap: ObjectCapability,
    kind: ObjectKind,
) -> Option<RetiredRecord> {
    retire_record(owner, cap, kind, |state| state == EntryState::Live)
}

/// Trusted payload teardown can also revoke an escrowed source. Its retained
/// backing pin outlives the entry, and late cancellation cannot revive it.
pub(crate) fn remove_for_teardown(
    owner: AddressSpaceId,
    cap: ObjectCapability,
    kind: ObjectKind,
) -> bool {
    remove_with_state(owner, cap, kind, |state| state != EntryState::Staged)
}

fn remove_with_state(
    owner: AddressSpaceId,
    cap: ObjectCapability,
    kind: ObjectKind,
    accepts: impl FnOnce(EntryState) -> bool,
) -> bool {
    let Some(retired) = retire_record(owner, cap, kind, accepts) else {
        return false;
    };
    // Only the local capability guard leaves here. Caller subsystem guards
    // and ordinary token cleanup remain separate outer-context work.
    retired.release();
    true
}

fn retire_record(
    owner: AddressSpaceId,
    cap: ObjectCapability,
    kind: ObjectKind,
    accepts: impl FnOnce(EntryState) -> bool,
) -> Option<RetiredRecord> {
    let mut tables = CAPABILITIES.lock();
    let table = tables.get_mut(&owner)?;
    let entry = table.objects.get(&cap)?;
    if entry.kind != kind || !accepts(entry.state) {
        return None;
    }
    table.objects.take(&cap).map(RetiredRecord::new)
}

/// Drop the complete authority namespace after subsystem payload teardown.
pub fn close_address_space(owner: AddressSpaceId) {
    let retired = {
        let mut tables = CAPABILITIES.lock();
        if let Some(table) = tables.get(&owner) {
            table.budget.retire();
        }
        tables.take(&owner)
    };
    if let Some(namespace) = retired {
        release_namespace(namespace);
    }
}

fn release_namespace(
    mut namespace: crate::klib::collections::retirement_list::RetiredEntry<(
        AddressSpaceId,
        AddressSpaceCapabilities,
    )>,
) {
    // Consume existing admitted storage one record at a time, without a
    // namespace snapshot or allocation. Partial abandonment retains the rest.
    while let Some((&cap, _)) = namespace.value().1.objects.first_key_value() {
        RetiredRecord::new(namespace.value_mut().1.objects.take(&cap).unwrap()).release();
    }
    namespace_tests::boundary(true);
    namespace.release();
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
