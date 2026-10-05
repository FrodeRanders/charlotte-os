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
/// syscall. Aggregate sponsorship limits are enforced by [`super::budget`]
/// in addition to this per-request bound.
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
    ResourceLimit,
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
    mappings: BTreeMap<AddressSpaceId, MemoryMappingState>,
    lend_state: LendState,
    dma_pins: usize,
    exclusive_dma_pins: usize,
    copy_pins: usize,
    destroy_when_unpinned: bool,
}

/// Staged or retired backing frames retain their charge until physical
/// release. Failure-path Drop rolls back partial allocation outside locks.
struct ChargedFrames {
    frames: Vec<PAddr>,
    charge: Option<super::budget::Charge>,
}

impl Drop for ChargedFrames {
    fn drop(&mut self) {
        self.free();
    }
}

impl ChargedFrames {
    fn free(&mut self) -> bool {
        if self.frames.is_empty() {
            return false;
        }
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        let mut failed = false;
        for frame in self.frames.drain(..) {
            failed |= allocator.deallocate_frame(frame).is_err();
        }
        drop(allocator);
        if failed {
            // Fail closed: a failed physical release must not create fresh
            // quota headroom. Retain the bounded ledger charge as quarantine.
            core::mem::forget(self.charge.take());
            crate::early_logln!("WARNING: failed to free memory-object frames; charge quarantined");
        }
        // The charge field drops after this body and the allocator lock.
        failed
    }

    fn release(mut self) -> Result<(), MemoryObjectError> {
        if self.free() {
            Err(MemoryObjectError::FrameFreeFailed)
        } else {
            Ok(())
        }
    }

    fn into_parts(mut self) -> (Vec<PAddr>, super::budget::Charge) {
        let frames = core::mem::take(&mut self.frames);
        (frames, self.charge.take().expect("staged charge missing"))
    }
}

fn allocate_frames(
    owner: super::AddressSpaceHandle,
    pages: usize,
) -> Result<ChargedFrames, MemoryObjectError> {
    let charge = super::budget::reserve_captured(
        owner,
        super::budget::Amount {
            pages: pages as u64,
            objects: 1,
        },
    )
    .map_err(|error| match error {
        super::budget::Error::StaleDomain => MemoryObjectError::AddressSpaceMissing,
        super::budget::Error::Limit => MemoryObjectError::ResourceLimit,
    })?;
    let mut staged = ChargedFrames {
        frames: Vec::new(),
        charge: Some(charge),
    };
    staged.frames.try_reserve_exact(pages).map_err(|_| MemoryObjectError::FrameAllocFailed)?;
    let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
    if !charlotte_lifecycle::resources::frames_available(
        allocator.free_frames() as u64,
        allocator.usable_bytes() / PAGE_SIZE as u64,
        pages as u64,
    ) {
        return Err(MemoryObjectError::ResourceLimit);
    }
    for _ in 0..pages {
        staged
            .frames
            .push(allocator.allocate_frame().map_err(|_| MemoryObjectError::FrameAllocFailed)?);
    }
    drop(allocator);
    Ok(staged)
}

struct ScopedCopyPin(Option<CopyPin>);
impl Drop for ScopedCopyPin {
    fn drop(&mut self) {
        if let Some(pin) = self.0.take() {
            unpin_copy(pin);
        }
    }
}

pub(crate) struct CopyPin {
    object: MemoryObjectId,
    frames: Vec<PAddr>,
}

pub(crate) struct DmaPin {
    object: MemoryObjectId,
    frames: Vec<PAddr>,
    exclusive: bool,
}

impl DmaPin {
    pub(crate) fn frames(&self) -> &[PAddr] {
        &self.frames
    }

    pub(crate) fn object_id(&self) -> u64 {
        self.object
    }
}

#[derive(Debug, Clone, Copy)]
struct MemoryMappingState {
    base: VAddr,
    writable: bool,
    /// This mapping owns its virtual range in the kernel-assigned scratch
    /// window and returns it only after unmapping and TLB invalidation.
    scratch: bool,
}

#[derive(Debug)]
enum LendState {
    None,
    /// A revocation has fenced all new access while mappings are removed
    /// without holding the registry across the shootdown.
    Revoking,
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
            LendState::Revoking => true,
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

    fn insert(&mut self, id: MemoryObjectCap, cap: MemoryCap) {
        assert!(self.caps.insert(id, cap).is_none(), "memory payload identity reused");
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
    let identity =
        super::current_address_space_handle(owner).ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let reservation = admit_capability(owner, identity)?;

    let staged = allocate_frames(identity, pages)?;

    // The frames are exclusively owned and not yet published, so zeroing them
    // does not require the IRQ-masking allocator lock held across the (up to
    // 64 MiB) memset.
    for frame in &staged.frames {
        let ptr: *mut u8 = (*frame).into();
        unsafe {
            core::ptr::write_bytes(ptr, 0, PAGE_SIZE);
        }
    }

    let mut registry = MEMORY_OBJECTS.lock();
    if !staged.charge.as_ref().unwrap().active() {
        return Err(MemoryObjectError::AddressSpaceMissing);
    }
    let cap = reservation.publish().map_err(capability_error)?;
    let (frames, charge) = staged.into_parts();
    let object_id = registry.next_object;
    registry.next_object = registry.next_object.checked_add(1).expect("memory object id overflow");
    registry.objects.insert(
        object_id,
        MemoryObject {
            owner,
            frames,
            charge,
            mappings: BTreeMap::new(),
            lend_state: LendState::None,
            dma_pins: 0,
            exclusive_dma_pins: 0,
            copy_pins: 0,
            destroy_when_unpinned: false,
        },
    );
    registry.caps_for_mut(owner).insert(
        cap,
        MemoryCap {
            object: object_id,
            rights: MemoryObjectRights::ALL,
        },
    );
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
    if matches!(object.lend_state, LendState::Revoking)
        || matches!(object.lend_state, LendState::Write { borrower, cap: lent }
            if borrower != asid || lent != cap)
    {
        return Err(MemoryObjectError::LendingActive);
    }
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
    if object.copy_pins != 0
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

/// Scratch allocation belongs to an address-space *lifetime*, not merely its
/// recyclable numeric ASID. A new generation starts again at the window base;
/// stale entries are harmless and are replaced on first use by the new owner.
#[derive(Debug)]
struct ScratchWindow {
    generation: usize,
    /// First byte never allocated from the window's high end.
    next: usize,
    /// Coalesced free extents, keyed by byte offset from the window base.
    free: BTreeMap<usize, usize>,
}

impl ScratchWindow {
    fn new(generation: usize) -> Self {
        Self {
            generation,
            next: 0,
            free: BTreeMap::new(),
        }
    }

    fn reserve(&mut self, bytes: usize) -> Result<usize, MemoryObjectError> {
        if let Some((offset, extent)) = self
            .free
            .iter()
            .find_map(|(offset, extent)| (*extent >= bytes).then_some((*offset, *extent)))
        {
            self.free.remove(&offset);
            if extent > bytes {
                self.free.insert(offset + bytes, extent - bytes);
            }
            return Ok(offset);
        }

        let offset = self.next;
        let end = offset.checked_add(bytes).ok_or(MemoryObjectError::OutOfScratch)?;
        if end > SCRATCH_WINDOW_SIZE {
            return Err(MemoryObjectError::OutOfScratch);
        }
        self.next = end;
        Ok(offset)
    }

    fn release(&mut self, mut offset: usize, mut bytes: usize) {
        if let Some((previous_offset, previous_bytes)) =
            self.free.range(..offset).next_back().map(|(offset, bytes)| (*offset, *bytes))
            && previous_offset + previous_bytes == offset
        {
            self.free.remove(&previous_offset);
            offset = previous_offset;
            bytes += previous_bytes;
        }

        while let Some(next_bytes) = self.free.remove(&(offset + bytes)) {
            bytes += next_bytes;
        }

        if offset + bytes == self.next {
            self.next = offset;
        } else {
            let replaced = self.free.insert(offset, bytes);
            debug_assert!(replaced.is_none(), "scratch extent released twice");
        }
    }
}

static SCRATCH_WINDOWS: crate::memory::LazyLock<
    crate::memory::Mutex<BTreeMap<AddressSpaceId, ScratchWindow>>,
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
    let mut windows = SCRATCH_WINDOWS.lock();
    let window = windows.entry(asid).or_insert_with(|| ScratchWindow::new(generation));
    if window.generation != generation {
        *window = ScratchWindow::new(generation);
    }
    let slot = window.reserve(bytes)?;
    Ok(VAddr::from(SCRATCH_WINDOW_BASE + (slot as u64)))
}

/// Return a scratch mapping's virtual range after its page-table entries have
/// been removed and the corresponding TLB invalidation has completed.
pub(crate) fn release_scratch(
    asid: AddressSpaceId,
    base: VAddr,
    pages: usize,
) -> Result<(), MemoryObjectError> {
    let base = <VAddr as Into<usize>>::into(base);
    let window_base = SCRATCH_WINDOW_BASE as usize;
    let offset = base.checked_sub(window_base).ok_or(MemoryObjectError::OutOfScratch)?;
    let bytes = pages.checked_mul(PAGE_SIZE).ok_or(MemoryObjectError::OutOfScratch)?;
    let end = offset.checked_add(bytes).ok_or(MemoryObjectError::OutOfScratch)?;
    if offset % PAGE_SIZE != 0 || bytes == 0 || end > SCRATCH_WINDOW_SIZE {
        return Err(MemoryObjectError::OutOfScratch);
    }

    let generation = crate::memory::current_address_space_handle(asid)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?
        .generation();
    let mut windows = SCRATCH_WINDOWS.lock();
    let window = windows.get_mut(&asid).ok_or(MemoryObjectError::OutOfScratch)?;
    if window.generation != generation {
        return Err(MemoryObjectError::AddressSpaceMissing);
    }
    window.release(offset, bytes);
    Ok(())
}

/// Forget all scratch allocation state when this exact address-space lifetime
/// is torn down. The caller holds `ADDRESS_SPACE_LIFECYCLE`, so a recycled ASID
/// cannot race this removal.
pub(crate) fn close_scratch_address_space(asid: AddressSpaceId) {
    SCRATCH_WINDOWS.lock().remove(&asid);
}

/// Map a memory object into the calling address space's scratch window at a
/// kernel-assigned virtual address and return it.
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
    let result = map_locked(asid, cap, base, writable, true);
    crate::cpu::isa::memory::tlb::inval_range_user(asid, base, pages);
    if let Err(error) = result {
        // If rollback itself failed, retaining the virtual range is safer than
        // aliasing a page-table entry that may still exist.
        if error != MemoryObjectError::UnmapFailed {
            let _ = release_scratch(asid, base, pages);
        }
    }
    result.map(|_| base)
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
    if !charlotte_launch::user_address::valid_pages(base.into(), 1) {
        return Err(MemoryObjectError::MapFailed);
    }

    // Serialize against address-space teardown/reuse for the complete map.
    let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
    let pages = {
        let registry = MEMORY_OBJECTS.lock();
        let cap_entry = registry.lookup(asid, cap)?;
        registry
            .objects
            .get(&cap_entry.object)
            .ok_or(MemoryObjectError::UnknownCapability)?
            .frames
            .len()
    };
    let result = map_locked(asid, cap, base, writable, false);
    crate::cpu::isa::memory::tlb::inval_range_user(asid, base, pages);
    result
}

/// Install a mapping while the caller holds `ADDRESS_SPACE_LIFECYCLE`.
fn map_locked(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    base: VAddr,
    writable: bool,
    scratch: bool,
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
        if !charlotte_launch::user_address::valid_pages(base.into(), object.frames.len()) {
            return Err(MemoryObjectError::MapFailed);
        }
        if object.destroy_when_unpinned {
            return Err(MemoryObjectError::LendingActive);
        }
        if object.exclusive_dma_pins != 0 || writable && object.copy_pins != 0 {
            return Err(MemoryObjectError::LendingActive);
        }
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
                scratch,
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
                        let mut cleanup_failed = false;
                        for cleanup_index in 0..mapped_pages {
                            let cleanup_vaddr = base + (cleanup_index * PAGE_SIZE);
                            cleanup_failed |= address_space.unmap_page(cleanup_vaddr).is_err();
                        }
                        result = Err(if cleanup_failed {
                            MemoryObjectError::UnmapFailed
                        } else {
                            MemoryObjectError::MapFailed
                        });
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
    unmap_serialized(asid, cap)
}

/// Caller holds lifecycle or IPC serialization. Retirement drains IPC before
/// memory cleanup, so its mapped-loan revocations must not re-enter lifecycle.
fn unmap_serialized(asid: AddressSpaceId, cap: MemoryObjectCap) -> Result<(), MemoryObjectError> {
    let (base, pages, scratch, result) = {
        let mut registry = MEMORY_OBJECTS.lock();
        let cap_entry = registry.lookup(asid, cap)?;
        let object = registry
            .objects
            .get_mut(&cap_entry.object)
            .ok_or(MemoryObjectError::UnknownCapability)?;
        let mapping = *object.mappings.get(&asid).ok_or(MemoryObjectError::NotMapped)?;
        let base = mapping.base;
        let pages = object.frames.len();

        let mut table = ADDRESS_SPACE_TABLE.lock();
        let result = match table.get_mut(asid) {
            Ok(address_space) => {
                let mut result = Ok(());
                for index in 0..pages {
                    let vaddr = base + (index * PAGE_SIZE);
                    if address_space.unmap_page(vaddr).is_err() {
                        result = Err(MemoryObjectError::UnmapFailed);
                        break;
                    }
                }
                result
            }
            Err(_) => Err(MemoryObjectError::AddressSpaceMissing),
        };
        if result.is_ok() {
            object.mappings.remove(&asid);
        }
        (base, pages, mapping.scratch, result)
    };
    crate::cpu::isa::memory::tlb::inval_range_user(asid, base, pages);
    if result.is_ok() && scratch {
        release_scratch(asid, base, pages)?;
    }
    result
}

/// Prepared authority never exposes an application-accessible destination.
/// One owner cancels staging, restores source authority, and releases backing.
#[must_use]
pub(crate) struct PreparedTransfer {
    target: AddressSpaceId,
    target_handle: super::AddressSpaceHandle,
    destination: crate::capability::Reservation,
    payload: PreparedPayload,
    committed: bool,
}

struct PreparedSource {
    owner: AddressSpaceId,
    handle: super::AddressSpaceHandle,
    cap: MemoryObjectCap,
    entry: MemoryCap,
    escrow: Option<crate::capability::SourceEscrow>,
    _pin: ScopedCopyPin,
}

enum PreparedPayload {
    Move {
        source: PreparedSource,
        rights: MemoryObjectRights,
    },
    Copy {
        frames: Option<ChargedFrames>,
    },
    Loan {
        source: PreparedSource,
        write: bool,
    },
}

impl PreparedPayload {
    fn source_mut(&mut self) -> Option<&mut PreparedSource> {
        match self {
            Self::Move {
                source,
                ..
            }
            | Self::Loan {
                source,
                ..
            } => Some(source),
            Self::Copy {
                ..
            } => None,
        }
    }
}

impl PreparedTransfer {
    pub(crate) fn target_cap(&self) -> MemoryObjectCap {
        self.destination.identity()
    }

    /// IPC adapter captures loan provenance before publishing the batch.
    pub(crate) fn loan_origin(&self) -> Option<(AddressSpaceId, MemoryObjectCap)> {
        match &self.payload {
            PreparedPayload::Loan {
                source,
                ..
            } => Some((source.owner, source.cap)),
            _ => None,
        }
    }

    pub(crate) fn commit(mut self) -> Result<MemoryObjectCap, MemoryObjectError> {
        commit_transfers(core::slice::from_mut(&mut self))?;
        Ok(self.target_cap())
    }
}

impl Drop for PreparedTransfer {
    fn drop(&mut self) {
        if !self.committed
            && let Some(source) = self.payload.source_mut()
            && let Some(escrow) = source.escrow.take()
        {
            // Restore only the captured source namespace, including a retiring
            // namespace awaiting payload drain. Never re-admit a fresh slot.
            let _ = escrow.rollback();
        }
        // Destination reservation drops before payload backing/pins. No live
        // destination, mapping or borrower state ever needs reversal.
    }
}

pub(crate) fn prepare_move(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
) -> Result<PreparedTransfer, MemoryObjectError> {
    prepare_source_transfer(owner, cap, target, None, false)
}

fn prepare_move_with_rights(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
    read_only: bool,
) -> Result<PreparedTransfer, MemoryObjectError> {
    prepare_source_transfer(owner, cap, target, None, read_only)
}

pub(crate) fn prepare_loan(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
    write: bool,
) -> Result<PreparedTransfer, MemoryObjectError> {
    if owner == target {
        return Err(MemoryObjectError::WrongOwner);
    }
    prepare_source_transfer(owner, cap, target, Some(write), false)
}

fn prepare_source_transfer(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
    loan: Option<bool>,
    read_only: bool,
) -> Result<PreparedTransfer, MemoryObjectError> {
    let source_handle =
        super::current_address_space_handle(owner).ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let target_handle = super::current_address_space_handle(target)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let mut registry = MEMORY_OBJECTS.lock();
    if !super::budget::accepting(source_handle) || !super::budget::accepting(target_handle) {
        return Err(MemoryObjectError::AddressSpaceMissing);
    }
    let entry = registry.lookup(owner, cap)?;
    let needed = match loan {
        Some(true) => MemoryObjectRights::MAP_WRITE,
        Some(false) => MemoryObjectRights::MAP_READ,
        None => MemoryObjectRights::TRANSFER,
    };
    if !entry.rights.contains(needed)
        || (read_only && !entry.rights.contains(MemoryObjectRights::MAP_READ))
    {
        return Err(MemoryObjectError::MissingRight);
    }
    let object = registry.objects.get(&entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    if object.owner != owner {
        return Err(MemoryObjectError::WrongOwner);
    }
    validate_source_transfer(object, target, loan, false)?;
    let pins = object.copy_pins.checked_add(1).ok_or(MemoryObjectError::ResourceLimit)?;
    let destination = admit_capability(target, target_handle)?;
    let escrow = crate::capability::escrow_captured(
        owner,
        cap,
        crate::capability::ObjectKind::Memory,
        Some(source_handle),
    )
    .map_err(capability_error)?;
    // No fallible work follows source escrow. A read-retention pin fences
    // writes/DMA and keeps backing alive even if source teardown drains it.
    registry.objects.get_mut(&entry.object).unwrap().copy_pins = pins;
    let source = PreparedSource {
        owner,
        handle: source_handle,
        cap,
        entry,
        escrow: Some(escrow),
        _pin: ScopedCopyPin(Some(CopyPin {
            object: entry.object,
            frames: Vec::new(),
        })),
    };
    let payload = match loan {
        Some(write) => PreparedPayload::Loan {
            source,
            write,
        },
        None => PreparedPayload::Move {
            source,
            rights: if read_only {
                MemoryObjectRights::MAP_READ
            } else {
                entry.rights
            },
        },
    };
    Ok(PreparedTransfer {
        target,
        target_handle,
        destination,
        payload,
        committed: false,
    })
}

fn validate_source_transfer(
    object: &MemoryObject,
    target: AddressSpaceId,
    loan: Option<bool>,
    prepared: bool,
) -> Result<(), MemoryObjectError> {
    if object.destroy_when_unpinned || object.dma_pins != 0 {
        return Err(MemoryObjectError::LendingActive);
    }
    if loan == Some(false) {
        if matches!(object.lend_state, LendState::Write { .. } | LendState::Revoking)
            || matches!(&object.lend_state, LendState::Read { borrowers }
                if borrowers.contains_key(&target))
        {
            return Err(MemoryObjectError::LendingActive);
        }
        if object.mappings.values().any(|mapping| mapping.writable) {
            return Err(MemoryObjectError::AlreadyMapped);
        }
        if prepared && object.copy_pins == 0 {
            return Err(MemoryObjectError::LendingActive);
        }
    } else {
        if object.lend_state.is_active() || object.copy_pins != usize::from(prepared) {
            return Err(MemoryObjectError::LendingActive);
        }
        if !object.mappings.is_empty() {
            return Err(MemoryObjectError::AlreadyMapped);
        }
    }
    Ok(())
}

/// Every mixed-mode destination publishes or none. Copies retain private
/// charged frames; loans retain source authority without live borrower state.
pub(crate) fn commit_transfers(
    transfers: &mut [PreparedTransfer],
) -> Result<(), MemoryObjectError> {
    use crate::capability::{
        Publication,
        SourceDisposition,
    };
    let mut authorities = Vec::new();
    authorities.try_reserve_exact(transfers.len()).map_err(|_| MemoryObjectError::ResourceLimit)?;
    let mut registry = MEMORY_OBJECTS.lock();
    for transfer in transfers.iter() {
        if transfer.committed || !super::budget::accepting(transfer.target_handle) {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        let (source, loan) = match &transfer.payload {
            PreparedPayload::Copy {
                frames,
            } => {
                if !frames.as_ref().unwrap().charge.as_ref().unwrap().active() {
                    return Err(MemoryObjectError::AddressSpaceMissing);
                }
                continue;
            }
            PreparedPayload::Move {
                source,
                ..
            } => (source, None),
            PreparedPayload::Loan {
                source,
                write,
            } => (source, Some(*write)),
        };
        if !super::budget::accepting(source.handle) {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        let stored = registry
            .caps
            .get(&source.owner)
            .and_then(|caps| caps.caps.get(&source.cap))
            .ok_or(MemoryObjectError::AddressSpaceMissing)?;
        let object = registry
            .objects
            .get(&source.entry.object)
            .ok_or(MemoryObjectError::AddressSpaceMissing)?;
        if stored.object != source.entry.object
            || stored.rights != source.entry.rights
            || object.owner != source.owner
        {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        validate_source_transfer(object, transfer.target, loan, true)?;
    }
    for transfer in transfers.iter_mut() {
        let source = match &mut transfer.payload {
            PreparedPayload::Move {
                source,
                ..
            } => Some((source.escrow.as_mut().unwrap(), SourceDisposition::Revoke)),
            PreparedPayload::Loan {
                source,
                ..
            } => Some((source.escrow.as_mut().unwrap(), SourceDisposition::Restore)),
            PreparedPayload::Copy {
                ..
            } => None,
        };
        authorities.push(Publication {
            destination: &mut transfer.destination,
            source,
        });
    }
    crate::capability::publish_batch(&mut authorities).map_err(capability_error)?;
    drop(authorities);
    // The registries serialize all verified payloads. No fallible work follows
    // authority publication; application memory lookup waits for this guard.
    for transfer in transfers.iter_mut() {
        let target_cap = transfer.target_cap();
        let entry = match &mut transfer.payload {
            PreparedPayload::Move {
                source,
                rights,
            } => {
                registry.caps.get_mut(&source.owner).unwrap().caps.remove(&source.cap);
                registry.objects.get_mut(&source.entry.object).unwrap().owner = transfer.target;
                MemoryCap {
                    object: source.entry.object,
                    rights: *rights,
                }
            }
            PreparedPayload::Copy {
                frames,
            } => {
                let (frames, charge) = frames.take().unwrap().into_parts();
                let object = registry.next_object;
                registry.next_object =
                    registry.next_object.checked_add(1).expect("memory object id overflow");
                registry.objects.insert(
                    object,
                    MemoryObject {
                        owner: transfer.target,
                        frames,
                        charge,
                        mappings: BTreeMap::new(),
                        lend_state: LendState::None,
                        dma_pins: 0,
                        exclusive_dma_pins: 0,
                        copy_pins: 0,
                        destroy_when_unpinned: false,
                    },
                );
                MemoryCap {
                    object,
                    rights: MemoryObjectRights::ALL,
                }
            }
            PreparedPayload::Loan {
                source,
                write,
            } => {
                let object = registry.objects.get_mut(&source.entry.object).unwrap();
                if *write {
                    object.lend_state = LendState::Write {
                        borrower: transfer.target,
                        cap: target_cap,
                    };
                } else {
                    if object.lend_state.is_none() {
                        object.lend_state = LendState::Read {
                            borrowers: BTreeMap::new(),
                        };
                    }
                    let LendState::Read {
                        borrowers,
                    } = &mut object.lend_state
                    else {
                        unreachable!()
                    };
                    borrowers.insert(transfer.target, target_cap);
                }
                MemoryCap {
                    object: source.entry.object,
                    rights: if *write {
                        MemoryObjectRights(
                            MemoryObjectRights::MAP_READ.0 | MemoryObjectRights::MAP_WRITE.0,
                        )
                    } else {
                        MemoryObjectRights::MAP_READ
                    },
                }
            }
        };
        registry.caps_for_mut(transfer.target).insert(target_cap, entry);
        transfer.committed = true;
    }
    Ok(())
}

pub fn move_to(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    prepare_move(owner, cap, target)?.commit()
}

/// Transfer an immutable launch object, attenuating its destination rights.
pub fn move_read_only_to(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    prepare_move_with_rights(owner, cap, target, true)?.commit()
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
    let mut registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(owner, cap)?;
    if !cap_entry.rights.contains(MemoryObjectRights::MAP_READ) {
        return Err(MemoryObjectError::MissingRight);
    }
    let object =
        registry.objects.get_mut(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    if object.owner != owner {
        return Err(MemoryObjectError::WrongOwner);
    }
    if matches!(object.lend_state, LendState::Write { .. } | LendState::Revoking)
        || object.dma_pins != 0
    {
        return Err(MemoryObjectError::LendingActive);
    }
    if object.mappings.values().any(|mapping| mapping.writable) {
        return Err(MemoryObjectError::AlreadyMapped);
    }
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(object.frames.len())
        .map_err(|_| MemoryObjectError::FrameAllocFailed)?;
    frames.extend_from_slice(&object.frames);
    object.copy_pins = object.copy_pins.checked_add(1).ok_or(MemoryObjectError::InvalidLength)?;
    Ok(CopyPin {
        object: cap_entry.object,
        frames,
    })
}

pub(crate) fn prepare_copy(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
) -> Result<PreparedTransfer, MemoryObjectError> {
    if owner == target {
        return Err(MemoryObjectError::WrongOwner);
    }
    let source_handle =
        super::current_address_space_handle(owner).ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let target_handle = super::current_address_space_handle(target)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let copy_pin = ScopedCopyPin(Some(pin_for_copy(owner, cap)?));
    if !super::budget::accepting(source_handle) {
        return Err(MemoryObjectError::AddressSpaceMissing);
    }
    let destination = admit_capability(target, target_handle)?;
    let source = copy_pin.0.as_ref().unwrap();
    // Private backing belongs to this owner, not to a destination registry
    // entry. Cancellation never consults a receiver-controlled numeric handle.
    let staged = allocate_frames(source_handle, source.frames.len())?;
    for (source, target_frame) in source.frames.iter().zip(staged.frames.iter()) {
        let source_ptr: *const u8 = (*source).into();
        let target_ptr: *mut u8 = (*target_frame).into();
        unsafe {
            core::ptr::copy_nonoverlapping(source_ptr, target_ptr, PAGE_SIZE);
        }
    }
    Ok(PreparedTransfer {
        target,
        target_handle,
        destination,
        payload: PreparedPayload::Copy {
            frames: Some(staged),
        },
        committed: false,
    })
}

pub fn copy_to(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    prepare_copy(owner, cap, target)?.commit()
}

pub fn lend_read(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    borrower: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    prepare_loan(owner, cap, borrower, false)?.commit()
}

pub fn lend_write(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    borrower: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    prepare_loan(owner, cap, borrower, true)?.commit()
}

pub fn revoke_lend(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    borrower: AddressSpaceId,
    borrower_cap: MemoryObjectCap,
) -> Result<(), MemoryObjectError> {
    let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
    revoke_lend_serialized(owner, cap, borrower, borrower_cap)
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
    revoke_lend_serialized(owner, cap, borrower, borrower_cap)
}

fn revoke_lend_serialized(
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

    match &object.lend_state {
        LendState::None => return Err(MemoryObjectError::NotLent),
        LendState::Revoking => return Err(MemoryObjectError::LendingActive),
        LendState::Read {
            borrowers,
        } => match borrowers.get(&borrower) {
            Some(cap) if *cap == borrower_cap => {}
            _ => return Err(MemoryObjectError::UnknownCapability),
        },
        LendState::Write {
            borrower: lent_to,
            cap: lent_cap,
        } => {
            if *lent_to != borrower || *lent_cap != borrower_cap {
                return Err(MemoryObjectError::UnknownCapability);
            }
        }
    }

    let mapped = object.mappings.contains_key(&borrower);
    let mut prior = core::mem::replace(&mut object.lend_state, LendState::Revoking);
    if mapped {
        drop(registry);
        let result = unmap_serialized(borrower, borrower_cap);
        registry = MEMORY_OBJECTS.lock();
        if let Err(error) = result {
            registry
                .objects
                .get_mut(&cap_entry.object)
                .expect("serialized revoke object disappeared")
                .lend_state = prior;
            return Err(error);
        }
    }

    let object =
        registry.objects.get_mut(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    object.lend_state = match &mut prior {
        LendState::Read {
            borrowers,
        } => {
            borrowers.remove(&borrower);
            if borrowers.is_empty() {
                LendState::None
            } else {
                prior
            }
        }
        LendState::Write {
            ..
        } => LendState::None,
        LendState::None | LendState::Revoking => unreachable!(),
    };
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
    if !crate::capability::contains(asid, cap, crate::capability::ObjectKind::Memory) {
        return Err(MemoryObjectError::UnknownCapability);
    }
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
        } else if object.lend_state.is_active() || object.dma_pins != 0 || object.copy_pins != 0 {
            registry.caps_for_mut(asid).caps.insert(cap, cap_entry);
            return Err(MemoryObjectError::LendingActive);
        } else if !object.mappings.is_empty() {
            registry.caps_for_mut(asid).caps.insert(cap, cap_entry);
            return Err(MemoryObjectError::AlreadyMapped);
        } else {
            true
        }
    };

    let backing = if should_destroy {
        let object = registry
            .objects
            .remove(&cap_entry.object)
            .ok_or(MemoryObjectError::UnknownCapability)?;
        Some(ChargedFrames {
            frames: object.frames,
            charge: Some(object.charge),
        })
    } else {
        None
    };
    let revoked = crate::capability::remove(asid, cap, crate::capability::ObjectKind::Memory);
    assert!(revoked, "memory payload capability was absent from unified table");
    drop(registry);
    backing.map_or(Ok(()), ChargedFrames::release)
}

pub fn close_address_space(asid: AddressSpaceId) {
    let mut frames_to_free = Vec::new();
    let mut invalidations = Vec::new();
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
            if registry
                .objects
                .get(&object_id)
                .is_some_and(|object| object.dma_pins != 0 || object.copy_pins != 0)
            {
                let object = registry.objects.get_mut(&object_id).unwrap();
                object.destroy_when_unpinned = true;
                for (mapped_asid, mapping) in core::mem::take(&mut object.mappings) {
                    let pages = object.frames.len();
                    let unmapped = unmap_pages(mapped_asid, mapping.base, pages).is_ok();
                    invalidations.push((
                        mapped_asid,
                        mapping.base,
                        pages,
                        mapping.scratch,
                        unmapped,
                    ));
                }
                continue;
            }
            if let Some(object) = registry.objects.remove(&object_id) {
                for (mapped_asid, mapping) in object.mappings {
                    let pages = object.frames.len();
                    let unmapped = unmap_pages(mapped_asid, mapping.base, pages).is_ok();
                    invalidations.push((
                        mapped_asid,
                        mapping.base,
                        pages,
                        mapping.scratch,
                        unmapped,
                    ));
                }
                remove_caps_for_object(&mut registry, object_id);
                frames_to_free.push(ChargedFrames {
                    frames: object.frames,
                    charge: Some(object.charge),
                });
            }
        }

        for object in registry.objects.values_mut() {
            if let Some(mapping) = object.mappings.remove(&asid) {
                let pages = object.frames.len();
                let unmapped = unmap_pages(asid, mapping.base, pages).is_ok();
                invalidations.push((asid, mapping.base, pages, mapping.scratch, unmapped));
            }
            match &mut object.lend_state {
                LendState::None | LendState::Revoking => {}
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
                    crate::capability::remove_for_teardown(
                        asid,
                        *cap,
                        crate::capability::ObjectKind::Memory,
                    ),
                    "memory payload capability was absent from unified table"
                );
            }
        }
    }

    // Mapping removal and frame reuse must be separated by a completed
    // cross-LP shootdown. An object owned by the closing domain may still have
    // mappings in other, live address spaces.
    for (mapped_asid, base, pages, scratch, unmapped) in invalidations {
        crate::cpu::isa::memory::tlb::inval_range_user(mapped_asid, base, pages);
        // The closing AS loses its complete scratch allocator below. Return
        // ranges mapped into other live domains so their windows do not leak.
        if mapped_asid != asid && scratch && unmapped {
            let _ = release_scratch(mapped_asid, base, pages);
        }
    }

    // Charges stay live through all shootdowns and final physical release.
    drop(frames_to_free);
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
                crate::capability::remove_for_teardown(
                    *asid,
                    cap_id,
                    crate::capability::ObjectKind::Memory,
                ),
                "memory payload capability was absent from unified table"
            );
        }
    }
}

/// Remove an object whose owner has exited once no DMA or copy operation can
/// still access its frames. The returned frames must be freed after releasing
/// the registry lock so the two IRQ-masking locks are never nested here.
fn take_deferred_frames_if_unpinned(
    registry: &mut MemoryObjectRegistry,
    object_id: MemoryObjectId,
) -> Option<ChargedFrames> {
    let should_destroy = registry.objects.get(&object_id).is_some_and(|object| {
        object.destroy_when_unpinned && object.dma_pins == 0 && object.copy_pins == 0
    });
    if !should_destroy {
        return None;
    }
    remove_caps_for_object(registry, object_id);
    registry.objects.remove(&object_id).map(|object| ChargedFrames {
        frames: object.frames,
        charge: Some(object.charge),
    })
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
    let frames = {
        let mut registry = MEMORY_OBJECTS.lock();
        release_copy_pin_locked(&mut registry, pin.object)
    };
    deallocate_frames(frames);
}

fn unmap_pages(asid: AddressSpaceId, base: VAddr, pages: usize) -> Result<(), MemoryObjectError> {
    let mut table = ADDRESS_SPACE_TABLE.lock();
    let address_space = table.get_mut(asid).map_err(|_| MemoryObjectError::AddressSpaceMissing)?;
    let mut failed = false;
    for index in 0..pages {
        failed |= address_space.unmap_page(base + (index * PAGE_SIZE)).is_err();
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
    if object.owner != asid {
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
    if object.destroy_when_unpinned
        || matches!(object.lend_state, LendState::Revoking)
        || device_writes && matches!(object.lend_state, LendState::Read { .. })
        || matches!(object.lend_state, LendState::Write { borrower, cap: lent }
            if borrower != asid || lent != cap)
    {
        return Err(MemoryObjectError::LendingActive);
    }
    if object.exclusive_dma_pins != 0
        || device_writes && object.copy_pins != 0
        || exclusive
            && (object.dma_pins != 0
                || object.copy_pins != 0
                || object.lend_state.is_active()
                || !object.mappings.is_empty())
    {
        return Err(MemoryObjectError::LendingActive);
    }
    let dma_pins = object.dma_pins.checked_add(1).ok_or(MemoryObjectError::InvalidLength)?;
    let exclusive_dma_pins = if exclusive {
        object.exclusive_dma_pins.checked_add(1).ok_or(MemoryObjectError::InvalidLength)?
    } else {
        object.exclusive_dma_pins
    };
    object.dma_pins = dma_pins;
    object.exclusive_dma_pins = exclusive_dma_pins;
    Ok(DmaPin {
        object: cap_entry.object,
        frames: object.frames.clone(),
        exclusive,
    })
}

pub(crate) fn unpin_dma(pin: DmaPin) {
    let frames = {
        let mut registry = MEMORY_OBJECTS.lock();
        let Some(object) = registry.objects.get_mut(&pin.object) else {
            return;
        };
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
