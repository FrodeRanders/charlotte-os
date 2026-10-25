//! Copy-pin admission, snapshot custody and ordered explicit unpin. Bare pin
//! abandonment retains backing; the existing scoped wrapper still actively
//! unpins. Source-escrow cancellation retains its separate registry context.
use super::{
    snapshot_storage::Preparing,
    *,
};

pub(super) struct ScopedCopyPin(Option<CopyPin>);
impl ScopedCopyPin {
    pub(super) fn new(pin: Option<CopyPin>) -> Self {
        Self(pin)
    }

    pub(super) fn as_ref(&self) -> Option<&CopyPin> {
        self.0.as_ref()
    }

    pub(super) fn take(&mut self) -> Option<CopyPin> {
        self.0.take()
    }
}

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

impl CopyPin {
    /// Assemble an already-admitted/fenced pin, including the source escrow's
    /// existing empty snapshot. This constructor performs no pin admission.
    pub(super) fn new(object: MemoryObjectId, frames: snapshot_storage::Snapshot) -> Self {
        Self {
            object,
            frames,
        }
    }

    pub(super) fn frames(&self) -> &[PAddr] {
        &self.frames
    }

    pub(super) fn object_id(&self) -> MemoryObjectId {
        self.object
    }
}

/// Hold a shared-read pin on a memory object while its frames are copied.
///
/// The pin keeps the frames alive and prevents every tracked path that can
/// grant or perform writes until [`unpin_copy`] releases it.
pub(crate) fn pin_for_copy(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
) -> Result<CopyPin, MemoryObjectError> {
    pin(owner, cap)
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
    prepare: impl FnOnce(
        super::super::AddressSpaceHandle,
        usize,
    ) -> Result<Preparing, MemoryObjectError>,
) -> Result<CopyPin, MemoryObjectError> {
    let handle = super::super::current_address_space_handle(owner)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let (object, pages) = {
        let registry = MEMORY_OBJECTS.lock();
        copy_admission(&registry, owner, cap)?
    };
    // No frame identity, Rust borrow or source pin crosses this interval.
    // Existing IPC/fixture contexts remain responsible for their outer policy.
    let mut prepared = prepare(handle, pages)?;
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
pub(super) fn release_copy_pin_locked(
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
