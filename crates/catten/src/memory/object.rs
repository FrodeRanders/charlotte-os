//! First-class kernel memory objects.
//!
//! This is the kernel-side ownership primitive that sitas-style userspace can
//! eventually name through capabilities. It deliberately stays below the
//! syscall ABI for now: callers pass kernel address-space ids, and the registry
//! enforces that moving an object consumes the sender's capability.

use alloc::vec::Vec;
use core::mem::ManuallyDrop;

use crate::{
    cpu::isa::interface::memory::{
        AddressSpaceInterface,
        address::Address,
    },
    klib::collections::retirement_list::{
        AdmittedMap,
        PreparedEntry,
        RetiredEntry,
        RetirementList,
    },
    memory::{
        ADDRESS_SPACE_LIFECYCLE,
        ADDRESS_SPACE_TABLE,
        AddressSpaceId,
        PHYSICAL_FRAME_ALLOCATOR,
        linear::{
            MemoryMapping,
            PageType,
            VAddr,
        },
        operation::{
            AddressSpaceOperation,
            OperationError,
        },
        physical::PAddr,
    },
};

const PAGE_SIZE: usize = 4096;
// Bound the stack metadata by processing identities in batches. Object size
// and admission limits are unchanged. Copy identities under the registry,
// then walk tables after releasing it.
const RETIREMENT_FRAME_BATCH: usize = 16;

mod backing;
mod borrower_storage_tests;
mod cap_storage;
mod copy_storage;
mod dma_storage;
mod mapping;
pub use mapping::{
    map,
    map_any,
    unmap,
};
mod snapshot_storage;
use backing::ChargedFrames;
pub(crate) use backing::RetiredMemory;
pub(crate) fn test_object_storage() {
    snapshot_storage::tests::run();
    backing::test_storage();
    borrower_storage_tests::run();
    cap_storage::tests::run();
    mapping::storage::tests::run();
    copy_storage::tests::run();
    dma_storage::tests::run();
    scratch_namespace::tests::run();
}
mod publication_tests;
pub(crate) fn test_publication_batch() {
    publication_tests::run();
}
pub(crate) mod namespace_close;
mod retirement;
pub(crate) use retirement::tests as retirement_tests;
use retirement::{
    MappingRetirementPin,
    RetiredMappings,
    RetiredObjectMappings,
};
mod revocation;
pub(crate) use revocation::LoanRevocation;
mod close;
mod transfer;
pub(crate) use transfer::{
    PreparedTransfer,
    commit_transfers,
    commit_transfers_with_authority,
    commit_undelivered_transfers,
    commit_undelivered_transfers_with_authority,
    deliver_memory,
    prepare_copy,
    prepare_loan,
    prepare_move,
};
use transfer::{
    commit_transfers_with_delivery,
    prepare_copy_with,
    prepare_source_transfer_with,
};
pub use transfer::{
    copy_to,
    lend_read,
    lend_write,
    move_read_only_to,
    move_to,
};

pub(crate) fn test_transfer_close_readiness(
    hold_ipc: fn(&mut dyn FnMut()),
    ipc_available: fn() -> bool,
) {
    close::tests::serialized(hold_ipc, ipc_available);
}

pub(crate) fn test_scheduled_transfer_close() {
    close::tests::scheduled();
}
mod scratch;
mod scratch_namespace;
use scratch::ScratchWindow;
pub(in crate::memory) use scratch_namespace::RetiredNamespace as RetiredScratch;

/// Upper bound on a single memory-object allocation, in pages (64 MiB). A
/// single `memory_alloc` cannot request an unbounded number of frames: this
/// caps the allocation loop and the amount of physical memory zeroed in one
/// syscall. Aggregate sponsorship limits are enforced by [`super::budget`]
/// in addition to this per-request bound.
pub const MAX_MEMORY_OBJECT_PAGES: usize =
    crate::resource_policy::POLICY.memory.object_pages as usize;

pub type MemoryObjectCap = u64;
type MemoryObjectId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryObjectError {
    UnknownCapability,
    WrongOwner,
    AlreadyMapped,
    NotMapped,
    InvalidLength,
    NotPageAligned,
    AddressSpaceMissing,
    MapFailed,
    UnmapFailed,
    InvalidationFailed(crate::cpu::isa::memory::tlb::InvalidationError),
    FrameAllocFailed,
    FrameFreeFailed,
    MissingRight,
    OutOfScratch,
    LendingActive,
    NotLent,
    ResourceLimit,
    AddressSpaceClosing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryObjectRights(u32);

impl MemoryObjectRights {
    pub const ALL: Self = Self(Self::MAP_READ.0 | Self::MAP_WRITE.0 | Self::TRANSFER.0);
    pub const MAP_READ: Self = Self(1 << 0);
    pub const MAP_WRITE: Self = Self(1 << 1);
    pub const TRANSFER: Self = Self(1 << 2);

    fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryObjectInfo {
    pub owner: AddressSpaceId,
    pub pages: usize,
    pub mapped: bool,
    pub lent: bool,
}

#[derive(Debug)]
struct MemoryObject {
    owner: AddressSpaceId,
    frames: Vec<PAddr>,
    charge: super::budget::Charge,
    mappings: mapping::storage::Mappings,
    lend_state: LendState,
    dma_pins: usize,
    exclusive_dma_pins: usize,
    copy_pins: usize,
    /// A detached mapping still owns this backing until its invalidation
    /// completes. An abandoned/failed retirement keeps this pin permanently.
    retirement_pins: usize,
    destroy_when_unpinned: bool,
    // Lazily admitted by a close waiter outside registry/lifecycle/IPC guards.
    // This exact transfer's source owns its list and deferred-disposal node.
    close_interest: Option<close::Source>,
    transfer_root: Option<super::AddressSpaceHandle>,
    retired_caps: cap_storage::RetiredCapabilities,
}

type ObjectStorage = PreparedEntry<(MemoryObjectId, MemoryObject)>;
type RetiredObject = RetiredEntry<(MemoryObjectId, MemoryObject)>;

struct ScopedCopyPin(Option<CopyPin>);
impl Drop for ScopedCopyPin {
    fn drop(&mut self) {
        if let Some(pin) = self.0.take() {
            unpin_copy(pin);
        }
    }
}

#[must_use]
pub(crate) struct CopyPin {
    object: MemoryObjectId,
    // A bare abandoned pin retains its snapshot and original backing fence.
    // Only explicit unpin may dispose the snapshot and decrement that fence.
    frames: snapshot_storage::Snapshot,
}

impl Drop for CopyPin {
    fn drop(&mut self) {
        // Explicit unpin owns release. Fallback retains both metadata and the
        // registry's original backing fence, without allocator/registry entry.
    }
}

#[must_use]
pub(crate) struct DmaPin {
    object: MemoryObjectId,
    frames: snapshot_storage::Snapshot,
    exclusive: bool,
}

impl Drop for DmaPin {
    fn drop(&mut self) {
        // Only confirmed backend cleanup or unused preparation permits explicit
        // unpin. Fallback retains snapshot, pin counters and original backing.
    }
}

impl DmaPin {
    pub(crate) fn frames(&self) -> &[PAddr] {
        &self.frames
    }

    pub(crate) fn object_id(&self) -> u64 {
        self.object
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemoryMappingState {
    address_space: super::AddressSpaceHandle,
    base: VAddr,
    /// Only this prefix was published. A failed map must not claim the
    /// foreign leaf at the page where installation stopped.
    installed_pages: usize,
    writable: bool,
    /// This mapping owns its virtual range in the kernel-assigned scratch
    /// window and returns it only after unmapping and TLB invalidation.
    scratch: bool,
}

/// Admitted map nodes also hold retirement leases during namespace cleanup.
/// Ordinary mappings have no lease: otherwise their own root could never close.
struct MemoryMappingRecord {
    state: MemoryMappingState,
    cleanup_lease: Option<AddressSpaceOperation>,
}

impl core::fmt::Debug for MemoryMappingRecord {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MemoryMappingRecord")
            .field("state", &self.state)
            .field("retaining_peer", &self.cleanup_lease.is_some())
            .finish()
    }
}

#[derive(Debug)]
enum LendState {
    None,
    /// A revocation has fenced all new access while mappings are removed
    /// without holding the registry across the shootdown.
    Revoking,
    Read {
        borrowers: AdmittedMap<AddressSpaceId, Borrower>,
    },
    Write {
        borrower: AddressSpaceId,
        cap: MemoryObjectCap,
    },
}

#[derive(Debug)]
struct Borrower {
    cap: MemoryObjectCap,
    // Confirmed revocation/namespace cleanup pairs detached authority with the
    // original borrower node. Its count cannot return before this node is freed.
    authority: Option<cap_storage::RetiredCapability>,
}

type BorrowerStorage = PreparedEntry<(AddressSpaceId, Borrower)>;
type RetiredBorrower = RetiredEntry<(AddressSpaceId, Borrower)>;

fn release_borrower(node: RetiredBorrower) {
    let mut authority = None;
    node.release_with(|(_, borrower)| authority = borrower.authority);
    if let Some(authority) = authority {
        authority.release();
    }
}

// The caller retains memory serialization and its existing root/physical proof.
// Remove both scalar payload and authority while that exact borrower is owned.
fn detach_borrower_authority(
    registry: &mut MemoryObjectRegistry,
    (asid, borrower): &mut (AddressSpaceId, Borrower),
) {
    assert!(borrower.authority.is_none());
    borrower.authority = Some(cap_storage::retire(registry, *asid, borrower.cap));
}

impl LendState {
    fn publish_read(
        &mut self,
        storage: BorrowerStorage,
        borrower: AddressSpaceId,
        cap: MemoryObjectCap,
    ) {
        if self.is_none() {
            *self = Self::Read {
                borrowers: AdmittedMap::new(),
            };
        }
        let Self::Read {
            borrowers,
        } = self
        else {
            unreachable!("read loan publication lost validated state")
        };
        borrowers.insert(
            storage,
            borrower,
            Borrower {
                cap,
                authority: None,
            },
        );
    }

    fn is_none(&self) -> bool {
        matches!(self, LendState::None)
    }

    fn is_active(&self) -> bool {
        !self.is_none()
    }

    fn references_cap(&self, asid: AddressSpaceId, cap: MemoryObjectCap) -> bool {
        match self {
            LendState::None => false,
            LendState::Revoking => true,
            LendState::Read {
                borrowers,
            } => borrowers.get(&asid).is_some_and(|lent| lent.cap == cap),
            LendState::Write {
                borrower,
                cap: lent,
            } => *borrower == asid && *lent == cap,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct MemoryCap {
    object: MemoryObjectId,
    rights: MemoryObjectRights,
    /// Owned by a PreparedTransfer until escrow and its pin finish together.
    transfer_in_flight: bool,
    /// IPC owns this move/copy until receive or reply observation.
    delivery_pending: bool,
}

#[derive(Debug)]
struct AddressSpaceCaps {
    handle: super::AddressSpaceHandle,
    caps: cap_storage::Caps,
}

#[derive(Debug)]
struct MemoryObjectRegistry {
    next_object: MemoryObjectId,
    objects: AdmittedMap<MemoryObjectId, MemoryObject>,
    caps: AdmittedMap<AddressSpaceId, AddressSpaceCaps>,
}

impl MemoryObjectRegistry {
    fn new() -> Self {
        Self {
            next_object: 1,
            objects: AdmittedMap::new(),
            caps: AdmittedMap::new(),
        }
    }

    fn allocate_object_id(&mut self) -> Result<MemoryObjectId, MemoryObjectError> {
        let id = self.next_object;
        self.next_object = id.checked_add(1).ok_or(MemoryObjectError::ResourceLimit)?;
        Ok(id)
    }

    fn lookup(
        &self,
        asid: AddressSpaceId,
        cap: MemoryObjectCap,
    ) -> Result<MemoryCap, MemoryObjectError> {
        let entry = self.lookup_for_cleanup(asid, cap)?;
        if entry.delivery_pending {
            return Err(MemoryObjectError::UnknownCapability);
        }
        Ok(entry)
    }

    /// IPC cleanup owns queued/unobserved authority even though applications
    /// cannot use it. Never use this lookup for application memory operations.
    fn lookup_for_cleanup(
        &self,
        asid: AddressSpaceId,
        cap: MemoryObjectCap,
    ) -> Result<MemoryCap, MemoryObjectError> {
        if !crate::capability::contains(asid, cap, crate::capability::ObjectKind::Memory) {
            return Err(MemoryObjectError::UnknownCapability);
        }
        self.caps
            .get(&asid)
            .and_then(|caps| caps.caps.get(&cap))
            .copied()
            .ok_or(MemoryObjectError::UnknownCapability)
    }
}

static MEMORY_OBJECTS: crate::memory::LazyLock<crate::memory::Mutex<MemoryObjectRegistry>> =
    crate::memory::LazyLock::new(|| crate::memory::Mutex::new(MemoryObjectRegistry::new()));

pub fn allocate(owner: AddressSpaceId, pages: usize) -> Result<MemoryObjectCap, MemoryObjectError> {
    if pages == 0 || pages > MAX_MEMORY_OBJECT_PAGES {
        return Err(MemoryObjectError::InvalidLength);
    }
    validate_address_space(owner)?;
    let identity =
        super::current_address_space_handle(owner).ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let reservation = cap_storage::Preparing::new(admit_capability(owner, identity)?, identity)?;

    let (reservation, cap) = reservation.try_prepare(|reservation| {
        let staged = ChargedFrames::allocate(identity, pages)?;

        // The frames are exclusively owned and not yet published, so zeroing them
        // does not require the IRQ-masking allocator lock held across the (up to
        // 64 MiB) memset.
        for frame in staged.prepared_frames() {
            let ptr: *mut u8 = (*frame).into();
            unsafe {
                core::ptr::write_bytes(ptr, 0, PAGE_SIZE);
            }
        }

        let mut registry = MEMORY_OBJECTS.lock();
        if !staged.admission_active() {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        reservation.validate(&registry)?;
        let object_id = registry.allocate_object_id()?;
        reservation.publish()?;
        let cap = reservation.identity();
        let (storage, frames, charge) = staged.into_parts();
        registry.objects.insert(
            storage,
            object_id,
            MemoryObject {
                owner,
                frames,
                charge,
                mappings: AdmittedMap::new(),
                lend_state: LendState::None,
                dma_pins: 0,
                exclusive_dma_pins: 0,
                copy_pins: 0,
                retirement_pins: 0,
                destroy_when_unpinned: false,
                close_interest: None,
                transfer_root: None,
                retired_caps: cap_storage::RetiredCapabilities::new(),
            },
        );
        reservation.install(
            &mut registry,
            MemoryCap {
                object: object_id,
                rights: MemoryObjectRights::ALL,
                transfer_in_flight: false,
                delivery_pending: false,
            },
        );
        Ok(cap)
    })?;
    reservation.finish();
    Ok(cap)
}

pub(crate) fn allocate_with_bytes(
    owner: AddressSpaceId,
    bytes: &[u8],
) -> Result<MemoryObjectCap, MemoryObjectError> {
    let cap = allocate(owner, bytes.len().max(1).div_ceil(PAGE_SIZE))?;
    let mut registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(owner, cap)?;
    let object =
        registry.objects.get_mut(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    let mut offset = 0usize;
    for frame in &object.frames {
        let count = (bytes.len() - offset).min(PAGE_SIZE);
        if count == 0 {
            break;
        }
        let destination: *mut u8 = (*frame).into();
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr().add(offset), destination, count);
        }
        offset += count;
    }
    Ok(cap)
}

pub fn info(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
) -> Result<MemoryObjectInfo, MemoryObjectError> {
    let registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(asid, cap)?;
    let object =
        registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    Ok(MemoryObjectInfo {
        owner: object.owner,
        pages: object.frames.len(),
        mapped: object.mappings.contains_key(&asid),
        lent: object.lend_state.is_active(),
    })
}

/// Copy `len` bytes from a readable memory object into kernel-owned memory.
///
/// This is used at trust boundaries such as executable loading: the complete
/// image is snapshotted before validation so userspace cannot mutate bytes
/// between ELF validation and segment mapping.
pub(crate) fn snapshot_bytes(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    len: usize,
) -> Result<Vec<u8>, MemoryObjectError> {
    let registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(asid, cap)?;
    if !cap_entry.rights.contains(MemoryObjectRights::MAP_READ) {
        return Err(MemoryObjectError::MissingRight);
    }
    let object =
        registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    if object.destroy_when_unpinned
        || object.retirement_pins != 0
        || matches!(object.lend_state, LendState::Revoking)
        || matches!(object.lend_state, LendState::Write { borrower, cap: lent }
            if borrower != asid || lent != cap)
    {
        return Err(MemoryObjectError::LendingActive);
    }
    if len == 0 || len > object.frames.len().saturating_mul(PAGE_SIZE) {
        return Err(MemoryObjectError::InvalidLength);
    }
    // Snapshot allocation is caller-sized; prepare it fallibly so a rejected
    // or oversized request cannot abort the kernel under heap pressure.
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).map_err(|_| MemoryObjectError::FrameAllocFailed)?;
    for frame in &object.frames {
        let remaining = len - bytes.len();
        if remaining == 0 {
            break;
        }
        let count = remaining.min(PAGE_SIZE);
        let source: *const u8 = (*frame).into();
        let old_len = bytes.len();
        bytes.resize(old_len + count, 0);
        unsafe {
            core::ptr::copy_nonoverlapping(source, bytes.as_mut_ptr().add(old_len), count);
        }
    }
    Ok(bytes)
}

/// Copy kernel-owned bytes into a writable memory object.
pub(crate) fn write_bytes(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    bytes: &[u8],
) -> Result<(), MemoryObjectError> {
    let registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(asid, cap)?;
    if !cap_entry.rights.contains(MemoryObjectRights::MAP_WRITE) {
        return Err(MemoryObjectError::MissingRight);
    }
    let object =
        registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    if object.destroy_when_unpinned
        || object.retirement_pins != 0
        || object.copy_pins != 0
        || matches!(object.lend_state, LendState::Read { .. } | LendState::Revoking)
        || matches!(object.lend_state, LendState::Write { borrower, cap: lent }
            if borrower != asid || lent != cap)
    {
        return Err(MemoryObjectError::LendingActive);
    }
    if bytes.is_empty() || bytes.len() > object.frames.len().saturating_mul(PAGE_SIZE) {
        return Err(MemoryObjectError::InvalidLength);
    }
    let mut copied = 0;
    for frame in &object.frames {
        let count = (bytes.len() - copied).min(PAGE_SIZE);
        if count == 0 {
            break;
        }
        let target: *mut u8 = (*frame).into();
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr().add(copied), target, count);
        }
        copied += count;
    }
    Ok(())
}

/// The per-address-space scratch window: a large *virtual* region (only
/// backed by physical frames while a cap is mapped into it) where
/// [`map_any`] assigns pages so services never hardcode scratch vaddrs.
/// The base sits well above the ELF load and heap of any user address
/// space; each AS has its own page table, so the same window base is valid
/// in every AS. 512 MiB gives the boot storm (every store-sourced service
/// ELF is mapped several times: buffer, transfer chunk, copy-back, hash)
/// comfortable headroom. Scratch virtual ranges are recycled after unmapping;
/// the size is therefore a bound on concurrently mapped scratch memory rather
/// than on the number of mappings performed during an address-space lifetime.
const SCRATCH_WINDOW_BASE: u64 = 0x0000_0000_4000_0000;
const SCRATCH_WINDOW_PAGES: usize = (512 * 1024 * 1024) / PAGE_SIZE;
const SCRATCH_WINDOW_SIZE: usize = SCRATCH_WINDOW_PAGES * PAGE_SIZE;

static SCRATCH_WINDOWS: crate::memory::LazyLock<
    crate::memory::Mutex<scratch_namespace::Namespaces>,
> = crate::memory::LazyLock::new(|| crate::memory::Mutex::new(AdmittedMap::new()));

/// Callers retain an exact operation/closing root or serialized fixture lifecycle.
/// The shared window prevents MMIO and memory mappings from colliding.
pub(crate) fn reserve_scratch(
    asid: AddressSpaceId,
    pages: usize,
) -> Result<VAddr, MemoryObjectError> {
    scratch_namespace::reserve(asid, pages)
}

/// Only confirmed unmap/invalidation (and the existing owner) permits release.
pub(crate) fn release_scratch(
    asid: AddressSpaceId,
    base: VAddr,
    pages: usize,
) -> Result<(), MemoryObjectError> {
    scratch_namespace::release(asid, base, pages)
}

pub(in crate::memory) fn detach_scratch(
    handle: super::AddressSpaceHandle,
) -> Option<RetiredScratch> {
    scratch_namespace::detach(handle)
}

pub(in crate::memory) fn scratch_registry_available() -> bool {
    SCRATCH_WINDOWS.try_lock().is_some()
}

pub(in crate::memory) fn scratch_admission_snapshot() -> ([u64; 2], [u64; 2]) {
    scratch_namespace::admission::snapshot()
}

pub(in crate::memory) fn test_with_scratch_locked(work: impl FnOnce()) {
    let _windows = SCRATCH_WINDOWS.lock();
    scratch_namespace::admission::with_locked(work);
}

/// Keep the exact address-space generation alive through a mapping operation,
/// then explicitly finish its lease on both success and ordinary error paths.
/// Panic or abandonment retains the generation rather than allowing premature
/// teardown.
fn with_address_space_operation<T>(
    asid: AddressSpaceId,
    operation: impl FnOnce() -> Result<T, MemoryObjectError>,
) -> Result<T, MemoryObjectError> {
    let handle = crate::memory::current_address_space_handle(asid)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let lease = AddressSpaceOperation::acquire(handle).map_err(|error| match error {
        OperationError::Closing => MemoryObjectError::AddressSpaceClosing,
        OperationError::Limit => MemoryObjectError::ResourceLimit,
        _ => MemoryObjectError::AddressSpaceMissing,
    })?;
    let result = operation();
    lease.release().map_err(|_| MemoryObjectError::AddressSpaceMissing)?;
    result
}

fn capability_error(error: crate::capability::AllocationError) -> MemoryObjectError {
    match error {
        crate::capability::AllocationError::Retired => MemoryObjectError::AddressSpaceMissing,
        crate::capability::AllocationError::UnknownCapability => {
            MemoryObjectError::UnknownCapability
        }
        _ => MemoryObjectError::ResourceLimit,
    }
}

fn admit_capability(
    owner: AddressSpaceId,
    identity: super::AddressSpaceHandle,
) -> Result<crate::capability::Reservation, MemoryObjectError> {
    crate::capability::reserve_captured(
        owner,
        crate::capability::ObjectKind::Memory,
        Some(identity),
    )
    .map_err(capability_error)
}

/// Hold a shared-read pin on a memory object while its frames are copied.
///
/// The pin keeps the frames alive and prevents every tracked path that can
/// grant or perform writes until [`unpin_copy`] releases it.
pub(crate) fn pin_for_copy(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
) -> Result<CopyPin, MemoryObjectError> {
    copy_storage::pin(owner, cap)
}

fn copy_admission(
    registry: &MemoryObjectRegistry,
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
) -> Result<(MemoryObjectId, usize), MemoryObjectError> {
    let cap_entry = registry.lookup(owner, cap)?;
    if !cap_entry.rights.contains(MemoryObjectRights::MAP_READ) {
        return Err(MemoryObjectError::MissingRight);
    }
    let object =
        registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    if object.owner != owner {
        return Err(MemoryObjectError::WrongOwner);
    }
    if object.destroy_when_unpinned
        || object.retirement_pins != 0
        || matches!(object.lend_state, LendState::Write { .. } | LendState::Revoking)
        || object.dma_pins != 0
    {
        return Err(MemoryObjectError::LendingActive);
    }
    if object.mappings.values().any(|mapping| mapping.state.writable) {
        return Err(MemoryObjectError::AlreadyMapped);
    }
    object.copy_pins.checked_add(1).ok_or(MemoryObjectError::InvalidLength)?;
    Ok((cap_entry.object, object.frames.len()))
}

pub fn revoke_lend(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    borrower: AddressSpaceId,
    borrower_cap: MemoryObjectCap,
) -> Result<(), MemoryObjectError> {
    revocation::revoke(owner, cap, borrower, borrower_cap)
}

/// IPC adapter boundary: every caller must retain the global IPC write guard
/// through this operation. Do not acquire lifecycle under that guard:
/// address-space retirement holds lifecycle and waits for IPC to drain.
pub(crate) fn revoke_lend_under_ipc(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    borrower: AddressSpaceId,
    borrower_cap: MemoryObjectCap,
) -> Result<(), MemoryObjectError> {
    revocation::revoke_serialized(owner, cap, borrower, borrower_cap)
}

pub fn close_cap(asid: AddressSpaceId, cap: MemoryObjectCap) -> Result<(), MemoryObjectError> {
    close::run(asid, cap, |interest| {
        crate::cpu::scheduler::try_park_until(
            interest,
            crate::cpu::scheduler::threads::MigrationConstraint::GeneralWait,
            || interest.completed(),
        )
        .map_err(|_| MemoryObjectError::ResourceLimit)
    })
}

/// Thread-context close and deterministic boot interleaving adapter. Callers
/// must not hold IPC/lifecycle/registry guards while a transfer can be live.
pub(crate) fn close_cap_with_wait(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    mut wait: impl FnMut(),
) -> Result<(), MemoryObjectError> {
    close::run(asid, cap, |_| {
        wait();
        Ok(())
    })
}

/// Nonwaiting raw kernel fixture adapter. Published IPC cleanup instead retains
/// try_retire_cap's owner and releases it after serialization. Busy rejection
/// consumes no authority; normal teardown retains pinned backing.
pub(crate) fn try_close_cap(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
) -> Result<(), MemoryObjectError> {
    let registry = MEMORY_OBJECTS.lock();
    if transfer_in_flight(&registry, asid, cap) {
        return Err(MemoryObjectError::LendingActive);
    }
    prepare_close_cap(registry, asid, cap)?.release()
}

/// IPC adapter: retire authority/payload under serialization, then retain this
/// owner until the caller releases every outer guard. No physical release here.
pub(crate) fn try_retire_cap(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
) -> Result<RetiredMemory, MemoryObjectError> {
    let registry = MEMORY_OBJECTS.lock();
    if transfer_in_flight(&registry, asid, cap) {
        return Err(MemoryObjectError::LendingActive);
    }
    prepare_close_cap(registry, asid, cap)
}

fn transfer_in_flight(
    registry: &MemoryObjectRegistry,
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
) -> bool {
    registry
        .caps
        .get(&asid)
        .and_then(|caps| caps.caps.get(&cap))
        .is_some_and(|entry| entry.transfer_in_flight)
}

fn prepare_close_cap(
    mut registry: lock_api::MutexGuard<
        '_,
        crate::cpu::multiprocessor::spin::mutex::MutexCore,
        MemoryObjectRegistry,
    >,
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
) -> Result<RetiredMemory, MemoryObjectError> {
    // Reject before mutating payload ownership. Reinserting a rejected cap
    // could allocate, so there must be no remove-and-restore cleanup ladder.
    let cap_entry = registry.lookup_for_cleanup(asid, cap)?;

    let should_destroy = {
        let object =
            registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
        if object.owner != asid {
            if object.lend_state.references_cap(asid, cap) {
                return Err(MemoryObjectError::LendingActive);
            }
            false
        } else if object.lend_state.is_active()
            || object.dma_pins != 0
            || object.copy_pins != 0
            || object.retirement_pins != 0
        {
            return Err(MemoryObjectError::LendingActive);
        } else if object.mappings.first_key_value().is_some() {
            return Err(MemoryObjectError::AlreadyMapped);
        } else {
            true
        }
    };

    let retired_cap = cap_storage::retire(&mut registry, asid, cap);
    let mut metadata = cap_storage::RetiredCapabilities::new();
    let backing = if should_destroy {
        let mut node =
            registry.objects.take(&cap_entry.object).expect("validated memory object missing");
        node.value_mut().1.retired_caps.push(retired_cap);
        Some(ChargedFrames::retired(node))
    } else {
        metadata.push(retired_cap);
        None
    };
    drop(registry);
    Ok(RetiredMemory::new(backing, metadata))
}

/// Detached mappings keep a separate registry pin: the last concurrent DMA
/// or copy unpin cannot release backing before this operation's shootdown.
/// No new metadata allocation is needed; existing admitted nodes move here.
///
/// Dropping an unfinished receipt deliberately leaves its pin in the registry
/// (and thus its physical backing and exact sponsorship charge quarantined).
// Detach only; callers carry metadata past local registry unlock. An active
// retirement pin keeps its exact borrower state fenced and unchanged.
fn clear_borrower(object: &mut MemoryObject, asid: AddressSpaceId) -> Option<RetiredBorrower> {
    if object.retirement_pins != 0 {
        return None;
    }
    match &mut object.lend_state {
        LendState::Read {
            borrowers,
        } => {
            let retired = borrowers.take(&asid);
            if borrowers.first_key_value().is_none() {
                object.lend_state = LendState::None;
            }
            retired
        }
        LendState::Write {
            borrower,
            ..
        } if *borrower == asid => {
            object.lend_state = LendState::None;
            None
        }
        _ => None,
    }
}

/// Raw boot fixtures serialize this adapter against address-space reuse.
/// Production root close uses namespace_close's retained generation owners.
pub(crate) fn close_address_space_fixture(asid: AddressSpaceId) {
    // Process one object at a time. Moving its existing mapping tree avoids
    // infallible Vec allocations of object IDs, invalidations and backing in
    // teardown. The receipt pin closes the registry-unlock/unpin race.
    let mut cursor = 0;
    loop {
        let receipt = {
            let registry = MEMORY_OBJECTS.lock();
            let next = registry.objects.iter().filter(|(id, _)| **id > cursor).find_map(
                |(&id, object)| {
                    ((object.owner == asid && !object.destroy_when_unpinned)
                        || object.mappings.contains_key(&asid))
                    .then_some(id)
                },
            );
            next.map(|id| RetiredObjectMappings::prepare(registry, id, asid))
        };
        match receipt {
            Some(receipt) => {
                cursor = receipt.object_id();
                receipt.detach_with(unmap_pages).finish(asid);
            }
            None => break,
        }
    }
    let mut registry = MEMORY_OBJECTS.lock();
    let mut borrowers = RetirementList::new();
    for object in registry.objects.values_mut() {
        if let Some(borrower) = clear_borrower(object, asid) {
            borrowers.push(borrower);
        }
    }
    for borrower in borrowers.iter_mut() {
        detach_borrower_authority(&mut registry, borrower);
    }
    let namespace = cap_storage::retire_namespace(&mut registry, asid);
    drop(registry);
    while let Some(borrower) = borrowers.pop() {
        release_borrower(borrower);
    }
    if let Some(namespace) = namespace {
        namespace.release();
    }
}

fn validate_address_space(asid: AddressSpaceId) -> Result<(), MemoryObjectError> {
    ADDRESS_SPACE_TABLE
        .lock()
        .get(asid)
        .map(|_| ())
        .map_err(|_| MemoryObjectError::AddressSpaceMissing)
}

/// Remove an object whose owner has exited once no DMA or copy operation can
/// still access its frames. The returned frames must be freed after releasing
/// the registry lock so the two IRQ-masking locks are never nested here.
fn take_deferred_frames_if_unpinned(
    registry: &mut MemoryObjectRegistry,
    object_id: MemoryObjectId,
) -> Option<ChargedFrames> {
    let should_destroy = registry.objects.get(&object_id).is_some_and(|object| {
        object.destroy_when_unpinned
            && object.dma_pins == 0
            && object.copy_pins == 0
            && object.retirement_pins == 0
            && object.mappings.first_key_value().is_none()
    });
    if !should_destroy {
        return None;
    }
    let metadata = cap_storage::retire_object(registry, object_id);
    let mut object = registry.objects.take(&object_id).expect("retiring memory object missing");
    object.value_mut().1.retired_caps.append(metadata);
    Some(ChargedFrames::retired(object))
}

fn release_copy_pin_locked(
    registry: &mut MemoryObjectRegistry,
    object_id: MemoryObjectId,
) -> Option<ChargedFrames> {
    let object = registry
        .objects
        .get_mut(&object_id)
        .expect("release_copy_pin: pinned memory object missing");
    object.copy_pins =
        object.copy_pins.checked_sub(1).expect("release_copy_pin: copy_pins underflow");
    take_deferred_frames_if_unpinned(registry, object_id)
}

fn deallocate_frames(frames: Option<ChargedFrames>) {
    drop(frames);
}

pub(crate) fn unpin_copy(pin: CopyPin) {
    unpin_copy_with(pin, || {});
}

fn unpin_copy_with(mut pin: CopyPin, snapshot_disposed: impl FnOnce()) {
    // Dispose metadata before its original pin can permit backing release.
    // The complete pin remains armed if disposal or its observer is interrupted.
    core::mem::take(&mut pin.frames).finish();
    snapshot_disposed();
    let frames = {
        let mut registry = MEMORY_OBJECTS.lock();
        release_copy_pin_locked(&mut registry, pin.object)
    };
    deallocate_frames(frames);
}

/// Check ownership before removing a leaf; a foreign replacement is not ours
/// to detach, even on an error path. A missing leaf is conservatively a failed
/// detach, so uncertain backing cannot be returned to the allocator.
fn unmap_owned_leaf(
    space: &mut crate::cpu::isa::memory::paging::AddressSpace,
    vaddr: VAddr,
    frame: PAddr,
) -> Result<(), MemoryObjectError> {
    if space.translate_address(vaddr).ok() != Some(frame) {
        return Err(MemoryObjectError::UnmapFailed);
    }
    match space.unmap_page(vaddr) {
        Ok(removed) if removed == frame => Ok(()),
        _ => Err(MemoryObjectError::UnmapFailed),
    }
}

fn unmap_pages(
    asid: AddressSpaceId,
    base: VAddr,
    frames: &[PAddr],
) -> Result<(), MemoryObjectError> {
    let mut table = ADDRESS_SPACE_TABLE.lock();
    let address_space = table.get_mut(asid).map_err(|_| MemoryObjectError::AddressSpaceMissing)?;
    let mut failed = false;
    for (index, &frame) in frames.iter().enumerate() {
        failed |= unmap_owned_leaf(address_space, base + (index * PAGE_SIZE), frame).is_err();
    }
    if failed {
        return Err(MemoryObjectError::UnmapFailed);
    }
    Ok(())
}

/// Return the physical base address of the first frame named by `cap`.
///
/// Physical addresses are kernel-private layout information, so this query is
/// restricted to the object's **owner**. A borrowed (read-only or writable)
/// capability is not sufficient authority: DMA drivers that need to address an
/// IPC-borrowed buffer must use the IOVA/DMA-domain path (`pin_for_dma` +
/// SMMU), never raw physical addresses. Returns 0 on any error.
pub fn get_phys(asid: AddressSpaceId, cap: MemoryObjectCap) -> u64 {
    get_phys_page(asid, cap, 0)
}

/// Return the physical address of one frame named by `cap`.
///
/// Memory-object frames are deliberately not assumed to be physically
/// contiguous. This owner-only compatibility query is not a DMA interface;
/// drivers must map owned or borrowed buffers through their DMA domain and use
/// the returned IOVA. Ownership-restricted like [`get_phys`].
pub fn get_phys_page(asid: AddressSpaceId, cap: MemoryObjectCap, page_index: usize) -> u64 {
    let registry = MEMORY_OBJECTS.lock();
    let Ok(cap_entry) = registry.lookup(asid, cap) else {
        return 0;
    };
    let Some(object) = registry.objects.get(&cap_entry.object) else {
        return 0;
    };
    if object.owner != asid || object.destroy_when_unpinned || object.retirement_pins != 0 {
        return 0;
    }
    object.frames.get(page_index).copied().map(<PAddr as Into<u64>>::into).unwrap_or(0)
}

pub(crate) fn pin_for_dma(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    device_reads: bool,
    device_writes: bool,
    exclusive: bool,
) -> Result<DmaPin, MemoryObjectError> {
    dma_storage::pin(
        asid,
        cap,
        dma_storage::Access {
            reads: device_reads,
            writes: device_writes,
            exclusive,
        },
    )
}

pub(crate) fn unpin_dma(pin: DmaPin) {
    unpin_dma_with(pin, || {});
}

fn unpin_dma_with(mut pin: DmaPin, snapshot_disposed: impl FnOnce()) {
    core::mem::take(&mut pin.frames).finish();
    snapshot_disposed();
    let frames = {
        let mut registry = MEMORY_OBJECTS.lock();
        let object =
            registry.objects.get_mut(&pin.object).expect("unpin_dma: pinned object missing");
        object.dma_pins = object.dma_pins.checked_sub(1).expect("unpin_dma: dma_pins underflow");
        if pin.exclusive {
            object.exclusive_dma_pins = object
                .exclusive_dma_pins
                .checked_sub(1)
                .expect("unpin_dma: exclusive_dma_pins underflow");
        }
        take_deferred_frames_if_unpinned(&mut registry, pin.object)
    };
    deallocate_frames(frames);
}
