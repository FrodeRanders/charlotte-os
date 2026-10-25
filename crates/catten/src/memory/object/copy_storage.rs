//! Snapshot factories leave memory serialization before any source pin exists.
//! Final revalidation fills preallocated storage and pins the exact object once.
use super::{
    snapshot_storage::Preparing,
    *,
};

fn publish(
    prepared: &mut Preparing,
    registry: &mut MemoryObjectRegistry,
    object: MemoryObjectId,
) -> CopyPin {
    let entry = registry.objects.get_mut(&object).expect("validated copy source missing");
    let pins = entry.copy_pins.checked_add(1).expect("validated copy pin overflow");
    let frames = prepared.fill_and_take(&entry.frames);
    entry.copy_pins = pins;
    CopyPin {
        object,
        frames,
    }
}

pub(super) fn pin(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
) -> Result<CopyPin, MemoryObjectError> {
    pin_with(owner, cap, Preparing::new)
}

fn pin_with(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    prepare: impl FnOnce(usize) -> Result<Preparing, MemoryObjectError>,
) -> Result<CopyPin, MemoryObjectError> {
    let handle = super::super::current_address_space_handle(owner)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let (object, pages) = {
        let registry = MEMORY_OBJECTS.lock();
        copy_admission(&registry, owner, cap)?
    };
    // No frame identity, Rust borrow or source pin crosses this interval.
    // Existing IPC/fixture contexts remain responsible for their outer policy.
    let mut prepared = prepare(pages)?;
    let result = (|| {
        let mut registry = MEMORY_OBJECTS.lock();
        if registry.caps.get(&owner).is_some_and(|namespace| namespace.handle != handle) {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        let (current, current_pages) = copy_admission(&registry, owner, cap)?;
        if current != object || current_pages != pages {
            return Err(MemoryObjectError::UnknownCapability);
        }
        Ok(publish(&mut prepared, &mut registry, object))
    })();
    prepared.finish();
    result
}

pub(super) mod tests;
