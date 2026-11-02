//! Retirement storage is admitted before attachment publication, never during
//! close. The existing queue/result receipt carries detached backing past IPC.
use core::{
    alloc::Layout,
    mem::ManuallyDrop,
};

use super::*;
use crate::{
    klib::admission::{
        IpcNodes,
        Reservation,
    },
    memory::object::{
        MemoryObjectError,
        RetiredMemory,
    },
};

type Charge = Reservation<IpcNodes, 2>;
#[derive(Debug, Default)]
struct Storage {
    caps: Vec<MemoryObjectCap>,
    retired: Vec<RetiredMemory>,
    // Ordinary constructor rejection frees capacity before refund.
    charge: Option<Charge>,
}
/// Complete queued/retired attachment metadata. Abandonment retains all fields.
#[must_use]
#[derive(Debug, Default)]
pub(super) struct MemoryAttachments(ManuallyDrop<Storage>);

impl core::ops::Deref for MemoryAttachments {
    type Target = [MemoryObjectCap];

    fn deref(&self) -> &Self::Target {
        &self.0.caps
    }
}

impl MemoryAttachments {
    pub(super) fn try_new(platform: bool, caps: Vec<MemoryObjectCap>) -> Result<Self, IpcError> {
        Self::try_new_with(platform, caps, |retired, count| {
            retired.try_reserve_exact(count).map_err(|_| IpcError::ResourceLimit)
        })
    }

    fn try_new_with(
        platform: bool,
        caps: Vec<MemoryObjectCap>,
        allocate: impl FnOnce(&mut Vec<RetiredMemory>, usize) -> Result<(), IpcError>,
    ) -> Result<Self, IpcError> {
        if caps.len() > CAP_VECTOR_MAX {
            return Err(IpcError::ResourceLimit);
        }
        let count = caps.len();
        let bytes = u64::try_from(
            Layout::array::<RetiredMemory>(count).map_err(|_| IpcError::ResourceLimit)?.size(),
        )
        .map_err(|_| IpcError::ResourceLimit)?;
        let mut storage = Storage {
            caps,
            retired: Vec::new(),
            charge: if count == 0 {
                None
            } else {
                Some(Charge::reserve(platform, [bytes, 0]).map_err(|_| IpcError::ResourceLimit)?)
            },
        };
        if count != 0 {
            allocate(&mut storage.retired, count)?;
        }
        if storage.retired.capacity() != count || !storage.retired.is_empty() {
            return Err(IpcError::ResourceLimit);
        }
        Ok(Self(ManuallyDrop::new(storage)))
    }

    pub(super) fn single(platform: bool, cap: MemoryObjectCap) -> Result<Self, IpcError> {
        let mut caps = Vec::new();
        caps.try_reserve_exact(1).map_err(|_| IpcError::ResourceLimit)?;
        caps.push(cap);
        Self::try_new(platform, caps)
    }

    /// IPC owns these exact undelivered caps; confirmed loans may already be
    /// absent. Application lookup prevents mappings/pins/escrow of owning caps.
    pub(super) fn retire(&mut self, asid: AddressSpaceId) {
        assert!(self.0.retired.is_empty(), "attachment retirement repeated");
        assert!(self.0.retired.capacity() >= self.0.caps.len());
        let storage = &mut *self.0;
        for &cap in &storage.caps {
            if let Some(owner) = retire_one(asid, cap) {
                storage.retired.push(owner);
            }
        }
    }

    pub(super) fn release_with(self, mut release: impl FnMut(RetiredMemory)) {
        struct Releasing {
            caps: Vec<MemoryObjectCap>,
            retired: alloc::vec::IntoIter<RetiredMemory>,
            charge: Option<Charge>,
        }
        let Storage {
            caps,
            retired,
            charge,
        } = ManuallyDrop::into_inner(self.0);
        // Preserve insertion order and every remaining owner across callback
        // interruption. Iterator exhaustion alone never refunds live capacity.
        let mut operation = ManuallyDrop::new(Releasing {
            caps,
            retired: retired.into_iter(),
            charge,
        });
        for owner in operation.retired.by_ref() {
            release(owner);
        }
        let Releasing {
            caps,
            retired,
            charge,
        } = ManuallyDrop::into_inner(operation);
        drop(caps);
        drop(retired);
        drop(charge);
    }

    /// Unused preparation or confirmed delivery leaves no detached owner here.
    /// The containing operation carries this storage out of IPC before disposal.
    pub(super) fn finish(self) {
        assert!(self.0.retired.is_empty(), "detached attachment owners remain");
        let Storage {
            caps,
            retired,
            charge,
        } = ManuallyDrop::into_inner(self.0);
        drop(caps);
        drop(retired);
        drop(charge);
    }
}

#[cfg(test)]
#[path = "attachments/host_tests.rs"]
mod host_tests;
#[cfg(feature = "kernel_tests")]
#[path = "attachments/tests.rs"]
mod tests;
#[cfg(feature = "kernel_tests")]
pub(super) use tests::run as test_admission;

pub(super) fn retire_one(asid: AddressSpaceId, cap: MemoryObjectCap) -> Option<RetiredMemory> {
    match crate::memory::object::try_retire_cap(asid, cap) {
        Ok(owner) => Some(owner),
        Err(MemoryObjectError::UnknownCapability) => None,
        Err(error) => panic!("undelivered IPC memory acquired live use: {error:?}"),
    }
}

pub(super) fn release_one(owner: RetiredMemory) {
    if owner.release().is_err() {
        // Authority is gone and no mapping/pin survives. Rejected physical
        // release quarantines the original charge; never retry partial frees.
        crate::logln!("[IPC] detached memory release rejected; original charge quarantined");
    }
}
