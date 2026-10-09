//! Non-DMA close retains root, descriptor and both metadata owners together.
//! Failed invalidation/abandonment retains the whole operation without cleanup.
use core::mem::ManuallyDrop;

use super::*;
use crate::klib::collections::retirement_list::RetiredEntry;

struct Resources {
    asid: AddressSpaceId,
    cap: DeviceCap,
    root: Option<AddressSpaceOperation>,
    object: Option<DeviceObject>,
    payload: Option<RetiredEntry<(DeviceCap, DeviceObject)>>,
    authority: Option<crate::capability::RetiredRecord>,
    started: bool,
    complete: bool,
}
struct PreparedClose(ManuallyDrop<Resources>);
impl PreparedClose {
    fn new(asid: AddressSpaceId, cap: DeviceCap) -> Result<Self, DeviceError> {
        let root = crate::memory::current_address_space_handle(asid)
            .map(|handle| AddressSpaceOperation::acquire(handle).map_err(operation_error))
            .transpose()?;
        Ok(Self(ManuallyDrop::new(Resources {
            asid,
            cap,
            root,
            object: None,
            payload: None,
            authority: None,
            started: false,
            complete: false,
        })))
    }

    fn claim(&mut self) -> Result<(), DeviceError> {
        let resources = &mut *self.0;
        let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let mut devices = DEVICES.lock();
        let caps = devices.get_mut(&resources.asid).ok_or(DeviceError::UnknownCapability)?;
        let object = caps.caps.get_mut(&resources.cap).ok_or(DeviceError::UnknownCapability)?;
        match object {
            DeviceObject::Mmio(region) => {
                if region.operation_in_flight
                    || reset_range_claimed(region.phys_base, region.pages * PAGE_SIZE)
                {
                    return Err(DeviceError::OperationInFlight);
                }
                region.operation_in_flight = true;
                resources.object = Some(*object);
                // The exact claimed descriptor stays reset-visible. Its charge
                // cannot disappear before invalidation/scratch completion.
            }
            DeviceObject::Interrupt(_) => resources.object = Some(*object),
            DeviceObject::DmaDomain {
                ..
            } => return Err(DeviceError::WrongType),
        }
        resources.authority = Some(
            crate::capability::detach(
                resources.asid,
                resources.cap,
                crate::capability::ObjectKind::Device,
            )
            .expect("device close authority was absent"),
        );
        if let Some(DeviceObject::Interrupt(irq)) = resources.object {
            resources.payload = Some(caps.caps.take(&resources.cap).unwrap());
            // Route removal stays serialized against new grants of this INTID.
            unroute_interrupt(irq.intid);
        }
        Ok(())
    }

    fn clean(&mut self) -> Result<(), DeviceError> {
        self.clean_with(
            arch_unmap,
            |asid, base, pages| {
                crate::cpu::isa::memory::tlb::try_inval_range_user(asid, base, pages).is_ok()
            },
            crate::memory::object::release_scratch,
        )
    }

    fn clean_with(
        &mut self,
        mut unmap: impl FnMut(AddressSpaceId, VAddr) -> Result<(), ()>,
        mut invalidate: impl FnMut(AddressSpaceId, VAddr, usize) -> bool,
        mut release: impl FnMut(
            AddressSpaceId,
            VAddr,
            usize,
        ) -> Result<(), crate::memory::object::MemoryObjectError>,
    ) -> Result<(), DeviceError> {
        let resources = &mut *self.0;
        assert!(resources.object.is_some() && !resources.started, "device close replay");
        resources.started = true;
        if let Some(DeviceObject::Mmio(region)) = resources.object {
            if let Some(base) = region.mapped {
                let mut failed = false;
                for index in 0..region.pages {
                    failed |= unmap(resources.asid, base + index * PAGE_SIZE).is_err();
                }
                failed |= !invalidate(resources.asid, base, region.pages);
                if !failed && region.scratch_mapped {
                    failed |= release(resources.asid, base, region.pages).is_err();
                }
                if failed {
                    // Terminal retention: original root, charge, scratch and
                    // claimed descriptor survive. Never reinsert or retry.
                    return Err(DeviceError::UnmapFailed);
                }
            }
            resources.payload = Some(
                DEVICES
                    .lock()
                    .get_mut(&resources.asid)
                    .expect("leased device namespace")
                    .caps
                    .take(&resources.cap)
                    .expect("claimed MMIO disappeared"),
            );
        }
        resources.complete = true;
        Ok(())
    }

    fn finish(mut self) -> Result<(), DeviceError> {
        assert!(self.0.object.is_none() || self.0.complete, "uncertain device close finished");
        if let Some(payload) = self.0.payload.take() {
            registry::release(payload);
        }
        if let Some(authority) = self.0.authority.take() {
            registry::release_authority(authority);
        }
        if let Some(root) = self.0.root.take() {
            root.release().map_err(operation_error)?;
        }
        Ok(())
    }
}

impl Drop for PreparedClose {
    fn drop(&mut self) {
        // ManuallyDrop retains exact root, detached metadata and claimed
        // descriptor. No retry, authority refund, hardware or allocator work.
    }
}

pub(super) fn run(
    asid: AddressSpaceId,
    cap: DeviceCap,
    after_claim: impl FnOnce(),
) -> Result<(), DeviceError> {
    let mut owner = PreparedClose::new(asid, cap)?;
    if let Err(error) = owner.claim() {
        // Ordinary admission failure precedes mutation and cleanup.
        owner.finish()?;
        return Err(error);
    }
    after_claim();
    owner.clean()?;
    owner.finish()
}

pub(super) mod tests;
