//! First-class kernel memory objects.
//!
//! This is the kernel-side ownership primitive that sitas-style userspace can
//! eventually name through capabilities. It deliberately stays below the
//! syscall ABI for now: callers pass kernel address-space ids, and the registry
//! enforces that moving an object consumes the sender's capability.

use alloc::{
    collections::BTreeMap,
    vec::Vec,
};

use crate::{
    cpu::isa::interface::memory::{
        AddressSpaceInterface,
        address::Address,
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
        physical::PAddr,
    },
};

const PAGE_SIZE: usize = 4096;

/// Upper bound on a single memory-object allocation, in pages (64 MiB). A
/// single `memory_alloc` cannot request an unbounded number of frames: this
/// caps the allocation loop and the amount of physical memory zeroed in one
/// syscall. Per-domain *total* quotas remain future work (see the manual's
/// known limitations); this bound only limits a single allocation.
pub const MAX_MEMORY_OBJECT_PAGES: usize = 16_384;

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
    FrameAllocFailed,
    FrameFreeFailed,
    MissingRight,
    OutOfScratch,
    LendingActive,
    NotLent,
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
    mappings: BTreeMap<AddressSpaceId, MemoryMappingState>,
    lend_state: LendState,
    dma_pins: usize,
    destroy_when_unpinned: bool,
}

#[cfg(target_arch = "aarch64")]
pub(crate) struct DmaPin {
    object: MemoryObjectId,
    frames: Vec<PAddr>,
}

#[cfg(target_arch = "aarch64")]
impl DmaPin {
    pub(crate) fn frames(&self) -> &[PAddr] {
        &self.frames
    }
}

#[derive(Debug, Clone, Copy)]
struct MemoryMappingState {
    base: VAddr,
    writable: bool,
}

#[derive(Debug)]
enum LendState {
    None,
    Read {
        borrowers: BTreeMap<AddressSpaceId, MemoryObjectCap>,
    },
    Write {
        borrower: AddressSpaceId,
        cap: MemoryObjectCap,
    },
}

impl LendState {
    fn is_none(&self) -> bool {
        matches!(self, LendState::None)
    }

    fn is_active(&self) -> bool {
        !self.is_none()
    }

    fn references_cap(&self, asid: AddressSpaceId, cap: MemoryObjectCap) -> bool {
        match self {
            LendState::None => false,
            LendState::Read {
                borrowers,
            } => borrowers.get(&asid).is_some_and(|lent| *lent == cap),
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
}

#[derive(Debug)]
struct AddressSpaceCaps {
    caps: BTreeMap<MemoryObjectCap, MemoryCap>,
}

impl AddressSpaceCaps {
    fn new() -> Self {
        Self {
            caps: BTreeMap::new(),
        }
    }

    fn insert(&mut self, owner: AddressSpaceId, cap: MemoryCap) -> MemoryObjectCap {
        let id = crate::capability::allocate(owner, crate::capability::ObjectKind::Memory);
        self.caps.insert(id, cap);
        id
    }
}

#[derive(Debug)]
struct MemoryObjectRegistry {
    next_object: MemoryObjectId,
    objects: BTreeMap<MemoryObjectId, MemoryObject>,
    caps: BTreeMap<AddressSpaceId, AddressSpaceCaps>,
}

impl MemoryObjectRegistry {
    fn new() -> Self {
        Self {
            next_object: 1,
            objects: BTreeMap::new(),
            caps: BTreeMap::new(),
        }
    }

    fn caps_for_mut(&mut self, asid: AddressSpaceId) -> &mut AddressSpaceCaps {
        self.caps.entry(asid).or_insert_with(AddressSpaceCaps::new)
    }

    fn lookup(
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

    let mut frames = Vec::new();
    {
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        for _ in 0..pages {
            match allocator.allocate_frame() {
                Ok(frame) => {
                    let ptr: *mut u8 = frame.into();
                    unsafe {
                        core::ptr::write_bytes(ptr, 0, PAGE_SIZE);
                    }
                    frames.push(frame);
                }
                Err(_) => {
                    for frame in frames.drain(..) {
                        allocator
                            .deallocate_frame(frame)
                            .map_err(|_| MemoryObjectError::FrameFreeFailed)?;
                    }
                    return Err(MemoryObjectError::FrameAllocFailed);
                }
            }
        }
    }

    let mut registry = MEMORY_OBJECTS.lock();
    let object_id = registry.next_object;
    registry.next_object += 1;
    registry.objects.insert(
        object_id,
        MemoryObject {
            owner,
            frames,
            mappings: BTreeMap::new(),
            lend_state: LendState::None,
            dma_pins: 0,
            destroy_when_unpinned: false,
        },
    );
    let cap = registry.caps_for_mut(owner).insert(
        owner,
        MemoryCap {
            object: object_id,
            rights: MemoryObjectRights::ALL,
        },
    );
    Ok(cap)
}

#[cfg(target_arch = "aarch64")]
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
#[cfg(target_arch = "aarch64")]
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
    if len == 0 || len > object.frames.len().saturating_mul(PAGE_SIZE) {
        return Err(MemoryObjectError::InvalidLength);
    }
    let mut bytes = Vec::with_capacity(len);
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
/// comfortable headroom; exhaustion is not a practical concern because the
/// window is virtual and pages are only committed while mapped.
const SCRATCH_WINDOW_BASE: u64 = 0x0000_0000_4000_0000;
const SCRATCH_WINDOW_PAGES: usize = (512 * 1024 * 1024) / PAGE_SIZE;
const SCRATCH_WINDOW_SIZE: usize = SCRATCH_WINDOW_PAGES * PAGE_SIZE;

/// Scratch allocation belongs to an address-space *lifetime*, not merely its
/// recyclable numeric ASID. A new generation starts again at the window base;
/// stale entries are harmless and are replaced on first use by the new owner.
static SCRATCH_WINDOW_NEXT: crate::memory::LazyLock<
    crate::memory::Mutex<BTreeMap<AddressSpaceId, (usize, usize)>>,
> = crate::memory::LazyLock::new(|| crate::memory::Mutex::new(BTreeMap::new()));

/// Reserve `pages` consecutive pages in an address space's scratch window at
/// a kernel-assigned virtual address. Shared with the device layer so MMIO
/// mappings come from the same window and can never collide with memory
/// mappings.
pub(crate) fn reserve_scratch(
    asid: AddressSpaceId,
    pages: usize,
) -> Result<VAddr, MemoryObjectError> {
    let bytes = pages.checked_mul(PAGE_SIZE).ok_or(MemoryObjectError::OutOfScratch)?;
    let generation = crate::memory::current_address_space_handle(asid)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?
        .generation();
    let mut windows = SCRATCH_WINDOW_NEXT.lock();
    let entry = windows.entry(asid).or_insert((generation, 0));
    if entry.0 != generation {
        *entry = (generation, 0);
    }
    let slot = entry.1;
    let end = slot.checked_add(bytes).ok_or(MemoryObjectError::OutOfScratch)?;
    if end > SCRATCH_WINDOW_SIZE {
        return Err(MemoryObjectError::OutOfScratch);
    }
    entry.1 = end;
    Ok(VAddr::from(SCRATCH_WINDOW_BASE + (slot as u64)))
}

/// Map a memory object into the calling address space's scratch window at a
/// kernel-assigned virtual address and return it. Pages are handed out
/// monotonically and never reused (the window is virtual, so exhaustion is
/// not a practical concern), which makes collisions impossible by
/// construction.
pub fn map_any(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    writable: bool,
) -> Result<VAddr, MemoryObjectError> {
    // The scratch reservation and page-table installation belong to the same
    // address-space lifetime. Otherwise teardown/reuse could occur between
    // them and apply the old generation's reservation to the new occupant.
    let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
    let pages = {
        let registry = MEMORY_OBJECTS.lock();
        let cap_entry = registry.lookup(asid, cap)?;
        let object =
            registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
        object.frames.len()
    };
    let base = reserve_scratch(asid, pages)?;
    map_locked(asid, cap, base, writable)?;
    Ok(base)
}

pub fn map(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    base: VAddr,
    writable: bool,
) -> Result<(), MemoryObjectError> {
    if !base.is_aligned_to(PAGE_SIZE) {
        return Err(MemoryObjectError::NotPageAligned);
    }

    // Serialize against address-space teardown/reuse for the complete map.
    let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
    map_locked(asid, cap, base, writable)
}

/// Install a mapping while the caller holds `ADDRESS_SPACE_LIFECYCLE`.
fn map_locked(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    base: VAddr,
    writable: bool,
) -> Result<(), MemoryObjectError> {
    let (object_id, frames, page_type) = {
        let mut registry = MEMORY_OBJECTS.lock();
        let cap_entry = registry.lookup(asid, cap)?;
        let required = if writable {
            MemoryObjectRights::MAP_WRITE
        } else {
            MemoryObjectRights::MAP_READ
        };
        if !cap_entry.rights.contains(required) {
            return Err(MemoryObjectError::MissingRight);
        }

        let object = registry
            .objects
            .get_mut(&cap_entry.object)
            .ok_or(MemoryObjectError::UnknownCapability)?;
        check_map_lend_state(object, asid, writable)?;
        if object.mappings.contains_key(&asid) {
            return Err(MemoryObjectError::AlreadyMapped);
        }
        let page_type = if writable {
            PageType::UserData
        } else {
            PageType::UserRoData
        };
        object.mappings.insert(
            asid,
            MemoryMappingState {
                base,
                writable,
            },
        );
        // Do not hold the memory-object registry while taking the address-space
        // table: teardown takes the table then the frame allocator, and
        // allocation takes the allocator then the registry, so holding the
        // registry across the table lock closes an AB-BC-CA deadlock cycle.
        (cap_entry.object, object.frames.clone(), page_type)
    };

    let map_result = {
        let mut mapped_pages = 0usize;
        let mut table = ADDRESS_SPACE_TABLE.lock();
        match table.get_mut(asid) {
            Ok(address_space) => {
                let mut result = Ok(());
                for (index, frame) in frames.iter().copied().enumerate() {
                    let vaddr = base + (index * PAGE_SIZE);
                    if address_space
                        .map_existing_page(MemoryMapping {
                            vaddr,
                            paddr: frame,
                            page_type,
                        })
                        .is_err()
                    {
                        for cleanup_index in 0..mapped_pages {
                            let cleanup_vaddr = base + (cleanup_index * PAGE_SIZE);
                            let _ = address_space.unmap_page(cleanup_vaddr);
                        }
                        result = Err(MemoryObjectError::MapFailed);
                        break;
                    }
                    mapped_pages += 1;
                }
                result
            }
            Err(_) => Err(MemoryObjectError::AddressSpaceMissing),
        }
    };
    if let Err(error) = map_result {
        let mut registry = MEMORY_OBJECTS.lock();
        if let Some(object) = registry.objects.get_mut(&object_id)
            && object.mappings.get(&asid).is_some_and(|mapping| mapping.base == base)
        {
            object.mappings.remove(&asid);
        }
        return Err(error);
    }
    Ok(())
}

pub fn unmap(asid: AddressSpaceId, cap: MemoryObjectCap) -> Result<(), MemoryObjectError> {
    let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
    let mut registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(asid, cap)?;
    let object =
        registry.objects.get_mut(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    let base = object.mappings.get(&asid).ok_or(MemoryObjectError::NotMapped)?.base;

    let mut table = ADDRESS_SPACE_TABLE.lock();
    let address_space = table.get_mut(asid).map_err(|_| MemoryObjectError::AddressSpaceMissing)?;
    for index in 0..object.frames.len() {
        let vaddr = base + (index * PAGE_SIZE);
        address_space.unmap_page(vaddr).map_err(|_| MemoryObjectError::UnmapFailed)?;
    }
    object.mappings.remove(&asid);
    Ok(())
}

pub fn move_to(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    validate_address_space(target)?;
    let mut registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(owner, cap)?;
    if !cap_entry.rights.contains(MemoryObjectRights::TRANSFER) {
        return Err(MemoryObjectError::MissingRight);
    }

    {
        let object =
            registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
        if object.owner != owner {
            return Err(MemoryObjectError::WrongOwner);
        }
        if object.lend_state.is_active() {
            return Err(MemoryObjectError::LendingActive);
        }
        if !object.mappings.is_empty() {
            return Err(MemoryObjectError::AlreadyMapped);
        }
    }

    registry.caps_for_mut(owner).caps.remove(&cap).ok_or(MemoryObjectError::UnknownCapability)?;
    registry
        .objects
        .get_mut(&cap_entry.object)
        .ok_or(MemoryObjectError::UnknownCapability)?
        .owner = target;
    let target_cap = registry.caps_for_mut(target).insert(
        target,
        MemoryCap {
            object: cap_entry.object,
            rights: cap_entry.rights,
        },
    );
    let revoked = crate::capability::remove(owner, cap, crate::capability::ObjectKind::Memory);
    assert!(revoked, "memory source capability was absent from unified table");
    Ok(target_cap)
}

/// Undo a successful [`move_to`] while preserving the owner's original
/// capability number. This is restricted to kernel-internal transaction
/// rollback; callers must supply the exact target capability returned by the
/// move and the now-vacant original capability slot.
pub(crate) fn rollback_move_to(
    target: AddressSpaceId,
    target_cap: MemoryObjectCap,
    owner: AddressSpaceId,
    original_cap: MemoryObjectCap,
) -> Result<(), MemoryObjectError> {
    let mut registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(target, target_cap)?;
    let object =
        registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    if object.owner != target || object.lend_state.is_active() || !object.mappings.is_empty() {
        return Err(MemoryObjectError::WrongOwner);
    }
    if registry.caps.get(&owner).is_some_and(|caps| caps.caps.contains_key(&original_cap)) {
        return Err(MemoryObjectError::LendingActive);
    }

    registry
        .caps
        .get_mut(&target)
        .and_then(|caps| caps.caps.remove(&target_cap))
        .ok_or(MemoryObjectError::UnknownCapability)?;
    let revoked =
        crate::capability::remove(target, target_cap, crate::capability::ObjectKind::Memory);
    assert!(revoked, "rollback target capability was absent from unified table");
    registry
        .objects
        .get_mut(&cap_entry.object)
        .ok_or(MemoryObjectError::UnknownCapability)?
        .owner = owner;
    registry.caps_for_mut(owner).caps.insert(original_cap, cap_entry);
    let restored =
        crate::capability::restore(owner, original_cap, crate::capability::ObjectKind::Memory);
    assert!(restored, "rollback source capability slot was not vacant");
    Ok(())
}

pub fn copy_to(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    if owner == target {
        return Err(MemoryObjectError::WrongOwner);
    }
    validate_address_space(target)?;

    let mut registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(owner, cap)?;
    if !cap_entry.rights.contains(MemoryObjectRights::MAP_READ) {
        return Err(MemoryObjectError::MissingRight);
    }

    let source_frames = {
        let object =
            registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
        if object.owner != owner {
            return Err(MemoryObjectError::WrongOwner);
        }
        if matches!(object.lend_state, LendState::Write { .. }) {
            return Err(MemoryObjectError::LendingActive);
        }
        if object.mappings.values().any(|mapping| mapping.writable) {
            return Err(MemoryObjectError::AlreadyMapped);
        }
        object.frames.clone()
    };

    let mut copied_frames = Vec::new();
    {
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        for source in source_frames {
            match allocator.allocate_frame() {
                Ok(frame) => {
                    let source_ptr: *const u8 = source.into();
                    let target_ptr: *mut u8 = frame.into();
                    unsafe {
                        core::ptr::copy_nonoverlapping(source_ptr, target_ptr, PAGE_SIZE);
                    }
                    copied_frames.push(frame);
                }
                Err(_) => {
                    for frame in copied_frames.drain(..) {
                        allocator
                            .deallocate_frame(frame)
                            .map_err(|_| MemoryObjectError::FrameFreeFailed)?;
                    }
                    return Err(MemoryObjectError::FrameAllocFailed);
                }
            }
        }
    }

    let object_id = registry.next_object;
    registry.next_object += 1;
    registry.objects.insert(
        object_id,
        MemoryObject {
            owner: target,
            frames: copied_frames,
            mappings: BTreeMap::new(),
            lend_state: LendState::None,
            dma_pins: 0,
            destroy_when_unpinned: false,
        },
    );
    let target_cap = registry.caps_for_mut(target).insert(
        target,
        MemoryCap {
            object: object_id,
            rights: MemoryObjectRights::ALL,
        },
    );
    Ok(target_cap)
}

pub fn lend_read(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    borrower: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    if owner == borrower {
        return Err(MemoryObjectError::WrongOwner);
    }
    validate_address_space(borrower)?;
    let mut registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(owner, cap)?;
    if !cap_entry.rights.contains(MemoryObjectRights::MAP_READ) {
        return Err(MemoryObjectError::MissingRight);
    }

    {
        let object =
            registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
        if object.owner != owner {
            return Err(MemoryObjectError::WrongOwner);
        }
        if matches!(object.lend_state, LendState::Write { .. }) {
            return Err(MemoryObjectError::LendingActive);
        }
        if let LendState::Read {
            borrowers,
        } = &object.lend_state
            && borrowers.contains_key(&borrower)
        {
            return Err(MemoryObjectError::LendingActive);
        }
        if object.mappings.values().any(|mapping| mapping.writable) {
            return Err(MemoryObjectError::AlreadyMapped);
        }
    }

    let borrower_cap = registry.caps_for_mut(borrower).insert(
        borrower,
        MemoryCap {
            object: cap_entry.object,
            rights: MemoryObjectRights::MAP_READ,
        },
    );
    let object =
        registry.objects.get_mut(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    match &mut object.lend_state {
        LendState::None => {
            let mut borrowers = BTreeMap::new();
            borrowers.insert(borrower, borrower_cap);
            object.lend_state = LendState::Read {
                borrowers,
            };
        }
        LendState::Read {
            borrowers,
        } => {
            borrowers.insert(borrower, borrower_cap);
        }
        LendState::Write {
            ..
        } => return Err(MemoryObjectError::LendingActive),
    }
    Ok(borrower_cap)
}

pub fn lend_write(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    borrower: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    if owner == borrower {
        return Err(MemoryObjectError::WrongOwner);
    }
    validate_address_space(borrower)?;
    let mut registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(owner, cap)?;
    if !cap_entry.rights.contains(MemoryObjectRights::MAP_WRITE) {
        return Err(MemoryObjectError::MissingRight);
    }

    {
        let object =
            registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
        if object.owner != owner {
            return Err(MemoryObjectError::WrongOwner);
        }
        if object.lend_state.is_active() {
            return Err(MemoryObjectError::LendingActive);
        }
        if !object.mappings.is_empty() {
            return Err(MemoryObjectError::AlreadyMapped);
        }
    }

    let borrower_cap = registry.caps_for_mut(borrower).insert(
        borrower,
        MemoryCap {
            object: cap_entry.object,
            rights: MemoryObjectRights(
                MemoryObjectRights::MAP_READ.0 | MemoryObjectRights::MAP_WRITE.0,
            ),
        },
    );
    registry
        .objects
        .get_mut(&cap_entry.object)
        .ok_or(MemoryObjectError::UnknownCapability)?
        .lend_state = LendState::Write {
        borrower,
        cap: borrower_cap,
    };
    Ok(borrower_cap)
}

pub fn revoke_lend(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    borrower: AddressSpaceId,
    borrower_cap: MemoryObjectCap,
) -> Result<(), MemoryObjectError> {
    let mut registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(owner, cap)?;
    let object =
        registry.objects.get_mut(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    if object.owner != owner {
        return Err(MemoryObjectError::WrongOwner);
    }

    let final_read_lend = match &mut object.lend_state {
        LendState::None => return Err(MemoryObjectError::NotLent),
        LendState::Read {
            borrowers,
        } => {
            match borrowers.get(&borrower) {
                Some(cap) if *cap == borrower_cap => {}
                _ => return Err(MemoryObjectError::UnknownCapability),
            }
            borrowers.remove(&borrower);
            borrowers.is_empty()
        }
        LendState::Write {
            borrower: lent_to,
            cap: lent_cap,
        } => {
            if *lent_to != borrower || *lent_cap != borrower_cap {
                return Err(MemoryObjectError::UnknownCapability);
            }
            true
        }
    };

    if object.mappings.contains_key(&borrower) {
        drop(registry);
        unmap(borrower, borrower_cap)?;
        registry = MEMORY_OBJECTS.lock();
    }

    let object =
        registry.objects.get_mut(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    if final_read_lend {
        object.lend_state = LendState::None;
    }
    registry
        .caps
        .get_mut(&borrower)
        .ok_or(MemoryObjectError::UnknownCapability)?
        .caps
        .remove(&borrower_cap)
        .ok_or(MemoryObjectError::UnknownCapability)?;
    let revoked =
        crate::capability::remove(borrower, borrower_cap, crate::capability::ObjectKind::Memory);
    assert!(revoked, "borrower capability was absent from unified table");
    Ok(())
}

pub fn close_cap(asid: AddressSpaceId, cap: MemoryObjectCap) -> Result<(), MemoryObjectError> {
    let mut registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry
        .caps
        .get_mut(&asid)
        .ok_or(MemoryObjectError::UnknownCapability)?
        .caps
        .remove(&cap)
        .ok_or(MemoryObjectError::UnknownCapability)?;

    let should_destroy = {
        let object =
            registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
        if object.owner != asid {
            if object.lend_state.references_cap(asid, cap) {
                registry.caps_for_mut(asid).caps.insert(cap, cap_entry);
                return Err(MemoryObjectError::LendingActive);
            }
            false
        } else if object.lend_state.is_active() || object.dma_pins != 0 {
            registry.caps_for_mut(asid).caps.insert(cap, cap_entry);
            return Err(MemoryObjectError::LendingActive);
        } else if !object.mappings.is_empty() {
            registry.caps_for_mut(asid).caps.insert(cap, cap_entry);
            return Err(MemoryObjectError::AlreadyMapped);
        } else {
            true
        }
    };

    if should_destroy {
        let object = registry
            .objects
            .remove(&cap_entry.object)
            .ok_or(MemoryObjectError::UnknownCapability)?;
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        for frame in object.frames {
            allocator.deallocate_frame(frame).map_err(|_| MemoryObjectError::FrameFreeFailed)?;
        }
    }
    let revoked = crate::capability::remove(asid, cap, crate::capability::ObjectKind::Memory);
    assert!(revoked, "memory payload capability was absent from unified table");
    Ok(())
}

pub fn close_address_space(asid: AddressSpaceId) {
    let mut frames_to_free = Vec::new();
    {
        let mut registry = MEMORY_OBJECTS.lock();
        let owned_objects = registry
            .objects
            .iter()
            .filter_map(|(object_id, object)| {
                if object.owner == asid {
                    Some(*object_id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        for object_id in owned_objects {
            if registry.objects.get(&object_id).is_some_and(|object| object.dma_pins != 0) {
                let object = registry.objects.get_mut(&object_id).unwrap();
                object.destroy_when_unpinned = true;
                for (mapped_asid, mapping) in core::mem::take(&mut object.mappings) {
                    let _ = unmap_pages(mapped_asid, mapping.base, object.frames.len());
                }
                continue;
            }
            if let Some(object) = registry.objects.remove(&object_id) {
                for (mapped_asid, mapping) in object.mappings {
                    let _ = unmap_pages(mapped_asid, mapping.base, object.frames.len());
                }
                remove_caps_for_object(&mut registry, object_id);
                frames_to_free.extend(object.frames);
            }
        }

        for object in registry.objects.values_mut() {
            if let Some(mapping) = object.mappings.remove(&asid) {
                let _ = unmap_pages(asid, mapping.base, object.frames.len());
            }
            match &mut object.lend_state {
                LendState::None => {}
                LendState::Read {
                    borrowers,
                } => {
                    borrowers.remove(&asid);
                    if borrowers.is_empty() {
                        object.lend_state = LendState::None;
                    }
                }
                LendState::Write {
                    borrower,
                    ..
                } if *borrower == asid => {
                    object.lend_state = LendState::None;
                }
                LendState::Write {
                    ..
                } => {}
            }
        }

        if let Some(caps) = registry.caps.remove(&asid) {
            for cap in caps.caps.keys() {
                assert!(
                    crate::capability::remove(asid, *cap, crate::capability::ObjectKind::Memory,),
                    "memory payload capability was absent from unified table"
                );
            }
        }
    }

    if !frames_to_free.is_empty() {
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        for frame in frames_to_free {
            let _ = allocator.deallocate_frame(frame);
        }
    }
}

fn check_map_lend_state(
    object: &MemoryObject,
    asid: AddressSpaceId,
    writable: bool,
) -> Result<(), MemoryObjectError> {
    if object.owner == asid {
        match object.lend_state {
            LendState::None => Ok(()),
            LendState::Read {
                ..
            } if !writable => Ok(()),
            _ => Err(MemoryObjectError::LendingActive),
        }
    } else {
        match &object.lend_state {
            LendState::Read {
                borrowers,
            } => {
                if borrowers.contains_key(&asid) && !writable {
                    Ok(())
                } else if borrowers.contains_key(&asid) {
                    Err(MemoryObjectError::MissingRight)
                } else {
                    Err(MemoryObjectError::WrongOwner)
                }
            }
            LendState::Write {
                borrower,
                ..
            } if *borrower == asid => Ok(()),
            _ => Err(MemoryObjectError::WrongOwner),
        }
    }
}

fn validate_address_space(asid: AddressSpaceId) -> Result<(), MemoryObjectError> {
    ADDRESS_SPACE_TABLE
        .lock()
        .get(asid)
        .map(|_| ())
        .map_err(|_| MemoryObjectError::AddressSpaceMissing)
}

fn remove_caps_for_object(registry: &mut MemoryObjectRegistry, object_id: MemoryObjectId) {
    for (asid, caps) in &mut registry.caps {
        let caps_to_remove = caps
            .caps
            .iter()
            .filter_map(|(cap_id, cap)| {
                if cap.object == object_id {
                    Some(*cap_id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for cap_id in caps_to_remove {
            caps.caps.remove(&cap_id);
            assert!(
                crate::capability::remove(*asid, cap_id, crate::capability::ObjectKind::Memory,),
                "memory payload capability was absent from unified table"
            );
        }
    }
}

fn unmap_pages(asid: AddressSpaceId, base: VAddr, pages: usize) -> Result<(), MemoryObjectError> {
    let mut table = ADDRESS_SPACE_TABLE.lock();
    let address_space = table.get_mut(asid).map_err(|_| MemoryObjectError::AddressSpaceMissing)?;
    for index in 0..pages {
        let _ = address_space.unmap_page(base + (index * PAGE_SIZE));
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
    if object.owner != asid {
        return 0;
    }
    object.frames.get(page_index).copied().map(<PAddr as Into<u64>>::into).unwrap_or(0)
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn pin_for_dma(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    device_reads: bool,
    device_writes: bool,
) -> Result<DmaPin, MemoryObjectError> {
    if !device_reads && !device_writes {
        return Err(MemoryObjectError::MissingRight);
    }
    let mut registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(asid, cap)?;
    if device_reads && !cap_entry.rights.contains(MemoryObjectRights::MAP_READ)
        || device_writes && !cap_entry.rights.contains(MemoryObjectRights::MAP_WRITE)
    {
        return Err(MemoryObjectError::MissingRight);
    }
    let object =
        registry.objects.get_mut(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    if object.destroy_when_unpinned {
        return Err(MemoryObjectError::LendingActive);
    }
    object.dma_pins = object.dma_pins.checked_add(1).ok_or(MemoryObjectError::InvalidLength)?;
    Ok(DmaPin {
        object: cap_entry.object,
        frames: object.frames.clone(),
    })
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn unpin_dma(pin: DmaPin) {
    let frames = {
        let mut registry = MEMORY_OBJECTS.lock();
        let Some(object) = registry.objects.get_mut(&pin.object) else {
            return;
        };
        debug_assert!(object.dma_pins != 0);
        object.dma_pins = object.dma_pins.saturating_sub(1);
        if object.dma_pins != 0 || !object.destroy_when_unpinned {
            return;
        }
        remove_caps_for_object(&mut registry, pin.object);
        registry.objects.remove(&pin.object).map(|object| object.frames)
    };
    if let Some(frames) = frames {
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        for frame in frames {
            let _ = allocator.deallocate_frame(frame);
        }
    }
}
