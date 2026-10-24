//! Scratch namespace/extent publication uses prepared nodes; whole namespace
//! retirement stays with the existing exact root metadata receipt.
use core::mem::ManuallyDrop;

use super::*;

type Storage = PreparedEntry<(AddressSpaceId, Namespace)>;
pub(super) type Namespaces = AdmittedMap<AddressSpaceId, Namespace>;
#[derive(Debug)]
pub(super) struct Namespace {
    handle: super::super::AddressSpaceHandle,
    window: ScratchWindow,
}
struct Resources {
    handle: super::super::AddressSpaceHandle,
    extent: Option<scratch::PreparedExtent>,
    namespace: Option<Storage>,
}
#[must_use]
struct Preparing(ManuallyDrop<Resources>);
impl Preparing {
    fn new(handle: super::super::AddressSpaceHandle) -> Result<Self, MemoryObjectError> {
        Self::with(handle, |_| Ok(()))
    }

    fn with(
        handle: super::super::AddressSpaceHandle,
        mut before: impl FnMut(u8) -> Result<(), MemoryObjectError>,
    ) -> Result<Self, MemoryObjectError> {
        let mut owner = Self(ManuallyDrop::new(Resources {
            handle,
            extent: None,
            namespace: None,
        }));
        let result = (|| {
            before(0)?;
            owner.0.extent = Some(
                scratch::PreparedExtent::try_new().map_err(|_| MemoryObjectError::ResourceLimit)?,
            );
            before(1)?;
            owner.0.namespace =
                Some(Storage::try_new().map_err(|_| MemoryObjectError::ResourceLimit)?);
            before(2)
        })();
        if let Err(error) = result {
            owner.finish();
            return Err(error);
        }
        Ok(owner)
    }

    fn publish(
        &mut self,
        windows: &mut Namespaces,
        bytes: usize,
    ) -> Result<usize, MemoryObjectError> {
        let handle = self.0.handle;
        if self.0.extent.is_none() {
            return Err(MemoryObjectError::ResourceLimit);
        }
        if let Some(namespace) = windows.get(&handle.id()) {
            if namespace.handle != handle {
                return Err(MemoryObjectError::AddressSpaceMissing);
            }
            namespace.window.first_fit(bytes).map_err(scratch_error)?;
        } else {
            // Fresh namespace admission precedes mutation too. The bounded
            // request was checked before allocation; this local check allocates nothing.
            ScratchWindow::new(SCRATCH_WINDOW_SIZE).first_fit(bytes).map_err(scratch_error)?;
            windows.insert(
                self.0.namespace.take().ok_or(MemoryObjectError::ResourceLimit)?,
                handle.id(),
                Namespace {
                    handle,
                    window: ScratchWindow::new(SCRATCH_WINDOW_SIZE),
                },
            );
        }
        windows
            .get_mut(&handle.id())
            .unwrap()
            .window
            .reserve(bytes, &mut self.0.extent)
            .map_err(scratch_error)
    }

    fn finish(mut self) {
        drop(self.0.namespace.take());
        drop(self.0.extent.take());
    }
}
fn scratch_error(error: scratch::Error) -> MemoryObjectError {
    match error {
        scratch::Error::AllocationFailed => MemoryObjectError::ResourceLimit,
        _ => MemoryObjectError::OutOfScratch,
    }
}
fn preflight(
    windows: &Namespaces,
    handle: super::super::AddressSpaceHandle,
    bytes: usize,
) -> Result<(), MemoryObjectError> {
    if let Some(namespace) = windows.get(&handle.id()) {
        if namespace.handle != handle {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        namespace.window.first_fit(bytes).map_err(scratch_error)?;
    }
    Ok(())
}
pub(super) fn reserve(asid: AddressSpaceId, pages: usize) -> Result<VAddr, MemoryObjectError> {
    reserve_with(asid, pages, Preparing::new)
}
fn reserve_with(
    asid: AddressSpaceId,
    pages: usize,
    prepare: impl FnOnce(super::super::AddressSpaceHandle) -> Result<Preparing, MemoryObjectError>,
) -> Result<VAddr, MemoryObjectError> {
    let bytes = pages
        .checked_mul(PAGE_SIZE)
        .filter(|bytes| *bytes != 0 && *bytes <= SCRATCH_WINDOW_SIZE)
        .ok_or(MemoryObjectError::OutOfScratch)?;
    // Existing callers retain an operation/closing root or serialized fixture
    // lifecycle. Never acquire lifecycle beneath a scratch/device guard.
    let handle = super::super::current_address_space_handle(asid)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    {
        preflight(&SCRATCH_WINDOWS.lock(), handle, bytes)?;
    }
    let mut prepared = prepare(handle)?;
    let result = { prepared.publish(&mut SCRATCH_WINDOWS.lock(), bytes) };
    prepared.finish();
    result.map(|offset| VAddr::from(SCRATCH_WINDOW_BASE + offset as u64))
}
pub(super) fn release(
    asid: AddressSpaceId,
    base: VAddr,
    pages: usize,
) -> Result<(), MemoryObjectError> {
    let offset = <VAddr as Into<usize>>::into(base)
        .checked_sub(SCRATCH_WINDOW_BASE as usize)
        .ok_or(MemoryObjectError::OutOfScratch)?;
    let bytes = pages.checked_mul(PAGE_SIZE).ok_or(MemoryObjectError::OutOfScratch)?;
    if offset % PAGE_SIZE != 0
        || bytes == 0
        || offset.checked_add(bytes).is_none_or(|end| end > SCRATCH_WINDOW_SIZE)
    {
        return Err(MemoryObjectError::OutOfScratch);
    }
    let handle = super::super::current_address_space_handle(asid)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let node = { take_extent(&mut SCRATCH_WINDOWS.lock(), handle, offset, bytes)? };
    node.release();
    Ok(())
}

fn take_extent(
    windows: &mut Namespaces,
    handle: super::super::AddressSpaceHandle,
    offset: usize,
    bytes: usize,
) -> Result<scratch::RetiredExtent, MemoryObjectError> {
    let namespace = windows.get_mut(&handle.id()).ok_or(MemoryObjectError::OutOfScratch)?;
    if namespace.handle != handle {
        return Err(MemoryObjectError::AddressSpaceMissing);
    }
    namespace.window.release(offset, bytes).map_err(scratch_error)
}

/// The whole root receipt retains every extent and its original namespace node
/// through final invalidation. Drop retains, never disposes beneath lifecycle.
#[must_use]
pub(in crate::memory) struct RetiredNamespace(RetiredEntry<(AddressSpaceId, Namespace)>);
impl RetiredNamespace {
    pub(in crate::memory) fn release(self) {
        super::super::retirement::metadata_tests::scratch_boundary();
        self.0.release_with(|(_, mut namespace)| namespace.window.release_all());
    }

    pub(in crate::memory) fn test_extent_count(&self) -> usize {
        self.0.value().1.window.extent_count()
    }

    pub(in crate::memory) fn test_assert_retained(&self, handle: super::super::AddressSpaceHandle) {
        assert_eq!(self.0.value().1.handle, handle);
        assert!(SCRATCH_WINDOWS.lock().get(&handle.id()).is_none());
    }
}
pub(super) fn detach(handle: super::super::AddressSpaceHandle) -> Option<RetiredNamespace> {
    detach_from(&mut SCRATCH_WINDOWS.lock(), handle)
}
fn detach_from(
    windows: &mut Namespaces,
    handle: super::super::AddressSpaceHandle,
) -> Option<RetiredNamespace> {
    let namespace = windows.get(&handle.id())?;
    if namespace.handle != handle {
        return None;
    }
    windows.take(&handle.id()).map(RetiredNamespace)
}

pub(super) mod tests;
