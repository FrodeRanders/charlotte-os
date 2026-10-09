//! Whole-domain device teardown borrows its exact closing root. Detached
//! registry storage is the work list; Drop retains unfinished records without
//! attempting hardware teardown or releasing authority under unknown guards.

use core::mem::ManuallyDrop;

use super::*;
use crate::{
    capability::{
        LifecycleGuard,
        ObjectKind,
    },
    cpu::isa::interface::memory::AddressSpaceInterface,
    memory::{
        AddressSpaceHandle,
        retirement::ClosingAddressSpace,
    },
};

#[must_use]
pub(crate) struct PreparedNamespaceDevices<'root> {
    root: &'root ClosingAddressSpace,
    objects: ManuallyDrop<BTreeMap<DeviceCap, DeviceObject>>,
}

/// Produced only after every detached device has completed physical cleanup
/// and released authority. A finish adapter cannot substitute an empty success.
#[must_use]
pub(crate) struct NamespaceDevicesClosed {
    handle: AddressSpaceHandle,
}

impl NamespaceDevicesClosed {
    pub(crate) fn handle(&self) -> AddressSpaceHandle {
        self.handle
    }
}

impl<'root> PreparedNamespaceDevices<'root> {
    /// The closing owner has drained old operations and retired sponsorship.
    /// Lifecycle precedes the device registry; no guard is retained in Self.
    pub(crate) fn prepare(
        root: &'root ClosingAddressSpace,
        _lifecycle: &LifecycleGuard<'_>,
    ) -> Result<Self, DeviceError> {
        let handle = root.device_cleanup_handle();
        let mut devices = DEVICES.lock();
        if devices.get(&handle.id()).is_some_and(|caps| {
            caps.caps.values().any(|object| {
                matches!(object, DeviceObject::Mmio(region) if region.operation_in_flight)
                    || matches!(
                        object,
                        DeviceObject::DmaDomain {
                            operation_in_flight: true,
                            ..
                        }
                    )
            })
        }) {
            return Err(DeviceError::OperationInFlight);
        }
        let objects = devices.remove(&handle.id()).map(|caps| caps.caps).unwrap_or_default();
        // Remove routes under the registry before another grant can reuse an
        // interrupt source. Hardware completion runs after both guards leave.
        for object in objects.values() {
            if let DeviceObject::Interrupt(irq) = object {
                unroute_interrupt(irq.intid);
            }
        }
        Ok(Self {
            root,
            objects: ManuallyDrop::new(objects),
        })
    }

    pub(crate) fn finish(self) -> Result<NamespaceDevicesClosed, DeviceError> {
        self.finish_with(
            unmap_owned_mmio,
            |handle, base, pages| {
                crate::cpu::isa::memory::tlb::try_inval_range_user(handle.id(), base, pages).is_ok()
            },
            crate::memory::object::release_scratch,
            dma::destroy_domain,
        )
    }

    fn finish_with(
        mut self,
        mut unmap: impl FnMut(AddressSpaceHandle, VAddr, PAddr) -> Result<(), ()>,
        mut invalidate: impl FnMut(AddressSpaceHandle, VAddr, usize) -> bool,
        mut release: impl FnMut(
            AddressSpaceId,
            VAddr,
            usize,
        ) -> Result<(), crate::memory::object::MemoryObjectError>,
        mut destroy: impl FnMut(u64) -> Result<(), dma::Error>,
    ) -> Result<NamespaceDevicesClosed, DeviceError> {
        let handle = self.root.device_cleanup_handle();
        while let Some((&cap, &object)) = self.objects.first_key_value() {
            match object {
                DeviceObject::Mmio(region) => {
                    if let Some(base) = region.mapped {
                        let mut detached = true;
                        for index in 0..region.pages {
                            detached &= unmap(
                                handle,
                                base + index * PAGE_SIZE,
                                PAddr::from((region.phys_base + index * PAGE_SIZE) as u64),
                            )
                            .is_ok();
                        }
                        // Even a failed detach may have removed a prefix. Keep
                        // the whole record/scratch claim until both are certain.
                        let quiescent = invalidate(handle, base, region.pages);
                        if !detached || !quiescent {
                            return Err(DeviceError::UnmapFailed);
                        }
                        if region.scratch_mapped {
                            release(handle.id(), base, region.pages)
                                .map_err(|_| DeviceError::UnmapFailed)?;
                        }
                    }
                }
                DeviceObject::Interrupt(_) => {}
                DeviceObject::DmaDomain {
                    id,
                    ..
                } => {
                    // Backends retain tables and memory pins on rejected
                    // hardware invalidation. Do not progress to loan/root cleanup.
                    destroy(id).map_err(|_| DeviceError::DmaInvalid)?;
                }
            }
            assert!(
                crate::capability::remove(handle.id(), cap, ObjectKind::Device),
                "retired device was absent from unified capability table"
            );
            self.objects.pop_first();
        }
        // Only ordinary confirmed completion destroys the admitted map storage.
        // There is no hardware work, allocation, or cleanup callback in Drop.
        unsafe { ManuallyDrop::drop(&mut self.objects) };
        Ok(NamespaceDevicesClosed {
            handle,
        })
    }
}

fn unmap_owned_mmio(handle: AddressSpaceHandle, base: VAddr, frame: PAddr) -> Result<(), ()> {
    let mut table = crate::memory::ADDRESS_SPACE_TABLE.lock();
    if table.generation(handle.id()).ok() != Some(handle.generation()) {
        return Err(());
    }
    let space = table.get_mut(handle.id()).map_err(|_| ())?;
    if space.translate_address(base).ok() != Some(frame) {
        return Err(());
    }
    match space.unmap_page(base) {
        Ok(removed) if removed == frame => Ok(()),
        _ => Err(()),
    }
}

pub(crate) mod tests;
