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
    next: MemoryObjectCap,
    caps: BTreeMap<MemoryObjectCap, MemoryCap>,
}

impl AddressSpaceCaps {
    fn new() -> Self {
        Self {
            next: 1,
            caps: BTreeMap::new(),
        }
    }

    fn insert(&mut self, cap: MemoryCap) -> MemoryObjectCap {
        let id = self.next;
        self.next += 1;
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
    if pages == 0 {
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
        },
    );
    let cap = registry.caps_for_mut(owner).insert(MemoryCap {
        object: object_id,
        rights: MemoryObjectRights::ALL,
    });
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

pub fn map(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    base: VAddr,
    writable: bool,
) -> Result<(), MemoryObjectError> {
    if !base.is_aligned_to(PAGE_SIZE) {
        return Err(MemoryObjectError::NotPageAligned);
    }

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

    let object =
        registry.objects.get_mut(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    check_map_lend_state(object, asid, writable)?;
    if object.mappings.contains_key(&asid) {
        return Err(MemoryObjectError::AlreadyMapped);
    }

    let page_type = if writable {
        PageType::UserData
    } else {
        PageType::UserRoData
    };
    let mut mapped_pages = 0usize;
    {
        let mut table = ADDRESS_SPACE_TABLE.lock();
        let address_space =
            table.get_mut(asid).map_err(|_| MemoryObjectError::AddressSpaceMissing)?;
        for (index, frame) in object.frames.iter().copied().enumerate() {
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
                return Err(MemoryObjectError::MapFailed);
            }
            mapped_pages += 1;
        }
    }
    object.mappings.insert(
        asid,
        MemoryMappingState {
            base,
            writable,
        },
    );
    Ok(())
}

pub fn unmap(asid: AddressSpaceId, cap: MemoryObjectCap) -> Result<(), MemoryObjectError> {
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
    let target_cap = registry.caps_for_mut(target).insert(MemoryCap {
        object: cap_entry.object,
        rights: cap_entry.rights,
    });
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
    registry
        .objects
        .get_mut(&cap_entry.object)
        .ok_or(MemoryObjectError::UnknownCapability)?
        .owner = owner;
    registry.caps_for_mut(owner).caps.insert(original_cap, cap_entry);
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
        },
    );
    let target_cap = registry.caps_for_mut(target).insert(MemoryCap {
        object: object_id,
        rights: MemoryObjectRights::ALL,
    });
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
        {
            if borrowers.contains_key(&borrower) {
                return Err(MemoryObjectError::LendingActive);
            }
        }
        if object.mappings.values().any(|mapping| mapping.writable) {
            return Err(MemoryObjectError::AlreadyMapped);
        }
    }

    let borrower_cap = registry.caps_for_mut(borrower).insert(MemoryCap {
        object: cap_entry.object,
        rights: MemoryObjectRights::MAP_READ,
    });
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

    let borrower_cap = registry.caps_for_mut(borrower).insert(MemoryCap {
        object: cap_entry.object,
        rights: MemoryObjectRights(
            MemoryObjectRights::MAP_READ.0 | MemoryObjectRights::MAP_WRITE.0,
        ),
    });
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
        } else if object.lend_state.is_active() {
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

        registry.caps.remove(&asid);
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
    for caps in registry.caps.values_mut() {
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

/// Return the physical base address of the first frame of memory object
/// `cap` owned by `asid`. Returns 0 on any error.
pub fn get_phys(asid: AddressSpaceId, cap: MemoryObjectCap) -> u64 {
    let registry = MEMORY_OBJECTS.lock();
    let Ok(cap_entry) = registry.lookup(asid, cap) else {
        return 0;
    };
    let object = registry.objects.get(&cap_entry.object);
    match object {
        Some(obj) if obj.owner == asid => {
            obj.frames.first().copied().map(|paddr| <PAddr as Into<u64>>::into(paddr)).unwrap_or(0)
        }
        _ => 0,
    }
}
