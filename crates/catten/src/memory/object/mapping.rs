//! Memory-object map publication and rollback/invalidation boundary. The caller's
//! exact operation lease and original retirement pin remain live through finish;
//! private preparation retains admitted nodes/snapshots until post-guard disposal.
use super::*;

pub(super) mod storage;
pub(super) mod tests;

/// Map a memory object into the calling address space's scratch window at a
/// kernel-assigned virtual address and return it.
pub fn map_any(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    writable: bool,
) -> Result<VAddr, MemoryObjectError> {
    with_address_space_operation(asid, || {
        map_any_with_lease(asid, cap, writable, crate::cpu::isa::memory::tlb::try_inval_range_user)
    })
}

fn map_any_with_lease(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    writable: bool,
    invalidate: impl FnOnce(
        AddressSpaceId,
        VAddr,
        usize,
    ) -> Result<(), crate::cpu::isa::memory::tlb::InvalidationError>,
) -> Result<VAddr, MemoryObjectError> {
    // The operation lease retains this exact generation through scratch,
    // page-table work and invalidation. The original pin fences all backing.
    let pages = {
        let registry = MEMORY_OBJECTS.lock();
        let cap_entry = registry.lookup(asid, cap)?;
        let object =
            registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
        object.frames.len()
    };
    let base = reserve_scratch(asid, pages)?;
    let mut pin = None;
    let result = map_locked(asid, cap, base, writable, true, &mut pin);
    finish_mapping(asid, base, pages, true, result, pin, invalidate).map(|()| base)
}

/// A missing pin means this call rejected before table access or mapping
/// publication. Only that branch may cancel unused scratch without maintenance.
/// Any admitted mapping, including a rejected/rolled-back prefix, still needs
/// confirmed invalidation before its original pin can be consumed.
fn finish_mapping(
    asid: AddressSpaceId,
    base: VAddr,
    pages: usize,
    scratch: bool,
    result: Result<(), MemoryObjectError>,
    pin: Option<MappingRetirementPin>,
    invalidate: impl FnOnce(
        AddressSpaceId,
        VAddr,
        usize,
    ) -> Result<(), crate::cpu::isa::memory::tlb::InvalidationError>,
) -> Result<(), MemoryObjectError> {
    if pin.is_some() {
        if let Err(cause) = invalidate(asid, base, pages) {
            // Keep scratch, installed/rollback metadata and backing pin. The
            // per-call cause diagnoses rejection; it cannot authorize replay.
            return Err(MemoryObjectError::InvalidationFailed(cause));
        }
    } else {
        assert!(result.is_err(), "successful mapping lost its backing pin");
    }
    if matches!(
        result,
        Err(MemoryObjectError::UnmapFailed | MemoryObjectError::InvalidationFailed(_))
    ) {
        return result;
    }
    let scratch_released = if scratch && result.is_err() {
        let released = release_scratch(asid, base, pages).is_ok();
        if !released {
            crate::early_logln!("[memory object] map rollback scratch release failed asid={asid}");
        }
        released
    } else {
        true
    };
    if scratch_released && let Some(pin) = pin {
        pin.release(None);
    }
    result
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
    if !charlotte_launch::user_address::valid_application_pages(base.into(), 1) {
        return Err(MemoryObjectError::MapFailed);
    }

    with_address_space_operation(asid, || {
        map_with_lease(
            asid,
            cap,
            base,
            writable,
            crate::cpu::isa::memory::tlb::try_inval_range_user,
        )
    })
}

fn map_with_lease(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    base: VAddr,
    writable: bool,
    invalidate: impl FnOnce(
        AddressSpaceId,
        VAddr,
        usize,
    ) -> Result<(), crate::cpu::isa::memory::tlb::InvalidationError>,
) -> Result<(), MemoryObjectError> {
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
    let mut pin = None;
    let result = map_locked(asid, cap, base, writable, false, &mut pin);
    finish_mapping(asid, base, pages, false, result, pin, invalidate)
}

/// Install a mapping while the caller owns an exact address-space operation.
pub(super) fn map_locked(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    base: VAddr,
    writable: bool,
    scratch: bool,
    pin: &mut Option<MappingRetirementPin>,
) -> Result<(), MemoryObjectError> {
    map_locked_with_cleanup(asid, cap, base, writable, scratch, pin, |space, vaddr, frame| {
        unmap_owned_leaf(space, vaddr, frame).is_ok()
    })
}

pub(super) fn map_locked_with_cleanup(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    base: VAddr,
    writable: bool,
    scratch: bool,
    pin: &mut Option<MappingRetirementPin>,
    cleanup: impl FnMut(&mut crate::cpu::isa::memory::paging::AddressSpace, VAddr, PAddr) -> bool,
) -> Result<(), MemoryObjectError> {
    map_locked_with_preparation(
        asid,
        cap,
        base,
        writable,
        scratch,
        pin,
        (cleanup, storage::Preparing::new, |_| {}),
    )
}

fn mapping_admission(
    registry: &MemoryObjectRegistry,
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    base: VAddr,
    writable: bool,
) -> Result<(MemoryObjectId, usize, PageType), MemoryObjectError> {
    let entry = registry.lookup(asid, cap)?;
    let required = if writable {
        MemoryObjectRights::MAP_WRITE
    } else {
        MemoryObjectRights::MAP_READ
    };
    if !entry.rights.contains(required) {
        return Err(MemoryObjectError::MissingRight);
    }
    let object = registry.objects.get(&entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    if !charlotte_launch::user_address::valid_application_pages(base.into(), object.frames.len()) {
        return Err(MemoryObjectError::MapFailed);
    }
    if object.destroy_when_unpinned
        || object.retirement_pins != 0
        || object.dma_pins != 0
        || writable && object.copy_pins != 0
    {
        return Err(MemoryObjectError::LendingActive);
    }
    check_map_lend_state(object, asid, writable)?;
    if object.mappings.contains_key(&asid) {
        return Err(MemoryObjectError::AlreadyMapped);
    }
    Ok((
        entry.object,
        object.frames.len(),
        if writable {
            PageType::UserData
        } else {
            PageType::UserRoData
        },
    ))
}

fn map_locked_with_preparation(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    base: VAddr,
    writable: bool,
    scratch: bool,
    pin: &mut Option<MappingRetirementPin>,
    (mut cleanup, prepare, before_dispose): (
        impl FnMut(&mut crate::cpu::isa::memory::paging::AddressSpace, VAddr, PAddr) -> bool,
        impl FnOnce(
            crate::memory::AddressSpaceHandle,
            usize,
        ) -> Result<storage::Preparing, MemoryObjectError>,
        impl FnOnce(&storage::Preparing),
    ),
) -> Result<(), MemoryObjectError> {
    assert!(pin.is_none(), "map preparation already owns a retirement pin");
    // Capture before the registry. The caller's exact lease (or raw fixture's
    // lifecycle guard) retains this generation through mapping publication.
    let address_space = crate::memory::current_address_space_handle(asid)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let (object_id, pages, _) = {
        let registry = MEMORY_OBJECTS.lock();
        mapping_admission(&registry, asid, cap, base, writable)?
    };
    // No frame identity/borrow or source pin crosses this factory interval.
    // The existing caller root stays retained. Recheck the exact object and
    // all admission predicates before filling its preallocated snapshot.
    let mut prepared = prepare(address_space, pages)?;
    let result = (|| {
        let page_type = {
            let mut registry = MEMORY_OBJECTS.lock();
            if registry.caps.get(&asid).is_some_and(|namespace| namespace.handle != address_space) {
                return Err(MemoryObjectError::AddressSpaceMissing);
            }
            let (current, current_pages, page_type) =
                mapping_admission(&registry, asid, cap, base, writable)?;
            if current != object_id || current_pages != pages {
                return Err(MemoryObjectError::UnknownCapability);
            }
            *pin = Some(prepared.publish(
                &mut registry,
                object_id,
                asid,
                MemoryMappingState {
                    address_space,
                    base,
                    installed_pages: 0,
                    writable,
                    scratch,
                },
            ));
            page_type
        };

        let mut mapped_pages = 0usize;
        let map_result = {
            let mut table = ADDRESS_SPACE_TABLE.lock();
            match table.get_mut(asid) {
                Ok(address_space) => {
                    let mut result = Ok(());
                    for (index, frame) in prepared.frames().iter().copied().enumerate() {
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
                            for (cleanup_index, &cleanup_frame) in
                                prepared.frames().iter().take(mapped_pages).enumerate()
                            {
                                let cleanup_vaddr = base + (cleanup_index * PAGE_SIZE);
                                cleanup_failed |=
                                    !cleanup(address_space, cleanup_vaddr, cleanup_frame);
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
        {
            let mut registry = MEMORY_OBJECTS.lock();
            if let Some(object) = registry.objects.get_mut(&object_id)
                && object.mappings.get(&asid).is_some_and(|mapping| mapping.state.base == base)
            {
                // Preserve the exact published prefix even when leaves rolled back:
                // invalidation has not yet confirmed that cached access ended.
                object.mappings.get_mut(&asid).unwrap().state.installed_pages = mapped_pages;
                if map_result.is_err() && map_result != Err(MemoryObjectError::UnmapFailed) {
                    // The pin owns this detached record through the caller's
                    // invalidation/scratch outcome. Failed maintenance retains it.
                    pin.as_mut().unwrap().retired_mapping = object.mappings.take(&asid);
                }
                // Failed cleanup leaves the original record in the registry. Its
                // pin fences close/transfer until confirmed completion (or forever
                // on abandonment), never authorizing a physical retry.
            }
        }
        map_result
    })();
    before_dispose(&prepared);
    prepared.finish();
    result
}

pub fn unmap(asid: AddressSpaceId, cap: MemoryObjectCap) -> Result<(), MemoryObjectError> {
    with_address_space_operation(asid, || unmap_serialized(asid, cap))
}

/// Caller retains either a live address-space operation or the global IPC
/// serialization. Retirement drains IPC before memory cleanup, so mapped-loan
/// revocation must not acquire lifecycle from underneath the IPC guard.
fn unmap_serialized(asid: AddressSpaceId, cap: MemoryObjectCap) -> Result<(), MemoryObjectError> {
    let (mapping, pages, mut pin) = {
        let mut registry = MEMORY_OBJECTS.lock();
        let cap_entry = registry.lookup(asid, cap)?;
        let object = registry
            .objects
            .get_mut(&cap_entry.object)
            .ok_or(MemoryObjectError::UnknownCapability)?;
        if object.retirement_pins != 0 {
            return Err(MemoryObjectError::LendingActive);
        }
        let mapping = object.mappings.get(&asid).ok_or(MemoryObjectError::NotMapped)?.state;
        let pages = object.frames.len();
        let pin = MappingRetirementPin::acquire(&mut registry, cap_entry.object);
        (mapping, pages, pin)
    };
    let result = pin.unmap_with(asid, mapping.base, mapping.installed_pages, unmap_pages);
    if let Err(cause) =
        crate::cpu::isa::memory::tlb::try_inval_range_user(asid, mapping.base, pages)
    {
        // Leaves may be detached, but neither scratch nor its pin may complete.
        return Err(MemoryObjectError::InvalidationFailed(cause));
    }
    if result.is_ok() && mapping.scratch {
        release_scratch(asid, mapping.base, pages)?;
    }
    if result.is_ok() {
        pin.retired_mapping =
            MEMORY_OBJECTS.lock().objects.get_mut(&pin.object).unwrap().mappings.take(&asid);
        assert!(pin.retired_mapping.is_some(), "confirmed unmap lost its mapping record");
        pin.release(None);
    }
    result
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
