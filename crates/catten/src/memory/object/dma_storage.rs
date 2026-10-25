//! DMA pin snapshots share inert capacity preparation with copy snapshots.
//! Hardware admission, maintenance and unpin authority remain in their typed owners.
use super::{
    snapshot_storage::Preparing,
    *,
};

#[derive(Clone, Copy)]
pub(super) struct Access {
    pub reads: bool,
    pub writes: bool,
    pub exclusive: bool,
}
impl Access {
    fn validate(self) -> Result<(), MemoryObjectError> {
        if !self.reads && !self.writes {
            Err(MemoryObjectError::MissingRight)
        } else {
            Ok(())
        }
    }
}

fn admission(
    registry: &MemoryObjectRegistry,
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    access: Access,
) -> Result<(MemoryObjectId, usize), MemoryObjectError> {
    access.validate()?;
    let entry = registry.lookup(asid, cap)?;
    if access.reads && !entry.rights.contains(MemoryObjectRights::MAP_READ)
        || access.writes && !entry.rights.contains(MemoryObjectRights::MAP_WRITE)
    {
        return Err(MemoryObjectError::MissingRight);
    }
    let object = registry.objects.get(&entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
    if object.destroy_when_unpinned
        || object.retirement_pins != 0
        || matches!(object.lend_state, LendState::Revoking)
        || access.writes && matches!(object.lend_state, LendState::Read { .. })
        || matches!(object.lend_state, LendState::Write { borrower, cap: lent }
            if borrower != asid || lent != cap)
    {
        return Err(MemoryObjectError::LendingActive);
    }
    if object.exclusive_dma_pins != 0
        || access.writes && object.copy_pins != 0
        || access.exclusive
            && (object.dma_pins != 0
                || object.copy_pins != 0
                || object.lend_state.is_active()
                || object.mappings.first_key_value().is_some())
    {
        return Err(MemoryObjectError::LendingActive);
    }
    object.dma_pins.checked_add(1).ok_or(MemoryObjectError::InvalidLength)?;
    if access.exclusive {
        object.exclusive_dma_pins.checked_add(1).ok_or(MemoryObjectError::InvalidLength)?;
    }
    Ok((entry.object, object.frames.len()))
}

fn publish(
    prepared: &mut Preparing,
    registry: &mut MemoryObjectRegistry,
    object: MemoryObjectId,
    access: Access,
) -> DmaPin {
    let entry = registry.objects.get_mut(&object).expect("validated DMA source missing");
    let pins = entry.dma_pins.checked_add(1).expect("validated DMA pin overflow");
    let exclusive = if access.exclusive {
        entry.exclusive_dma_pins.checked_add(1).expect("validated exclusive DMA pin overflow")
    } else {
        entry.exclusive_dma_pins
    };
    let frames = prepared.fill_and_take(&entry.frames);
    entry.dma_pins = pins;
    entry.exclusive_dma_pins = exclusive;
    DmaPin {
        object,
        frames,
        exclusive: access.exclusive,
    }
}

pub(super) fn pin(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    access: Access,
) -> Result<DmaPin, MemoryObjectError> {
    pin_with(asid, cap, access, Preparing::new)
}

fn pin_with(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    access: Access,
    prepare: impl FnOnce(
        super::super::AddressSpaceHandle,
        usize,
    ) -> Result<Preparing, MemoryObjectError>,
) -> Result<DmaPin, MemoryObjectError> {
    access.validate()?;
    let handle = super::super::current_address_space_handle(asid)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let (object, pages) = {
        let registry = MEMORY_OBJECTS.lock();
        admission(&registry, asid, cap, access)?
    };
    // The existing public DMA operation retains its exact root/capability;
    // serialized adapters keep their own caller policy. No source frame or pin
    // crosses this factory interval, before any backend domain/engine claim.
    let mut prepared = prepare(handle, pages)?;
    let result = (|| {
        let mut registry = MEMORY_OBJECTS.lock();
        if registry.caps.get(&asid).is_some_and(|namespace| namespace.handle != handle) {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        let (current, current_pages) = admission(&registry, asid, cap, access)?;
        if current != object || current_pages != pages {
            return Err(MemoryObjectError::UnknownCapability);
        }
        Ok(publish(&mut prepared, &mut registry, object, access))
    })();
    prepared.finish();
    result
}

pub(super) mod tests;
