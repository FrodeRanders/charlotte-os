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
        address::Address,
        AddressSpaceInterface,
    },
    memory::{
        linear::{
            MemoryMapping,
            PageType,
            VAddr,
        },
        physical::PAddr,
        AddressSpaceId,
        ADDRESS_SPACE_TABLE,
        PHYSICAL_FRAME_ALLOCATOR,
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
}

#[derive(Debug)]
struct MemoryObject {
    owner: AddressSpaceId,
    frames: Vec<PAddr>,
    mappings: BTreeMap<AddressSpaceId, VAddr>,
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
    if object.owner != asid {
        return Err(MemoryObjectError::WrongOwner);
    }
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
    object.mappings.insert(asid, base);
    Ok(())
}

pub fn unmap(asid: AddressSpaceId, cap: MemoryObjectCap) -> Result<(), MemoryObjectError> {
    let mut registry = MEMORY_OBJECTS.lock();
    let cap_entry = registry.lookup(asid, cap)?;
    let object =
        registry.objects.get_mut(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    let base = *object.mappings.get(&asid).ok_or(MemoryObjectError::NotMapped)?;

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
            false
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
