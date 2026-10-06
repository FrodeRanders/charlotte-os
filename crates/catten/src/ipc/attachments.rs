//! Retirement storage is admitted before attachment publication, never during
//! close. The existing queue/result receipt carries detached backing past IPC.
use super::*;
use crate::memory::object::{
    MemoryObjectError,
    RetiredMemory,
};

#[derive(Debug, Default)]
pub(super) struct MemoryAttachments {
    caps: Vec<MemoryObjectCap>,
    retired: Vec<RetiredMemory>,
}

impl core::ops::Deref for MemoryAttachments {
    type Target = [MemoryObjectCap];

    fn deref(&self) -> &Self::Target {
        &self.caps
    }
}

impl MemoryAttachments {
    pub(super) fn try_new(caps: Vec<MemoryObjectCap>) -> Result<Self, IpcError> {
        if caps.len() > CAP_VECTOR_MAX {
            return Err(IpcError::ResourceLimit);
        }
        let mut retired = Vec::new();
        retired.try_reserve_exact(caps.len()).map_err(|_| IpcError::ResourceLimit)?;
        Ok(Self {
            caps,
            retired,
        })
    }

    pub(super) fn single(cap: MemoryObjectCap) -> Result<Self, IpcError> {
        let mut caps = Vec::new();
        caps.try_reserve_exact(1).map_err(|_| IpcError::ResourceLimit)?;
        caps.push(cap);
        Self::try_new(caps)
    }

    /// IPC owns these exact undelivered caps; confirmed loans may already be
    /// absent. Application lookup prevents mappings/pins/escrow of owning caps.
    pub(super) fn retire(&mut self, asid: AddressSpaceId) {
        assert!(self.retired.is_empty(), "attachment retirement repeated");
        assert!(self.retired.capacity() >= self.caps.len());
        for &cap in &self.caps {
            if let Some(owner) = retire_one(asid, cap) {
                self.retired.push(owner);
            }
        }
    }

    pub(super) fn release_with(self, mut release: impl FnMut(RetiredMemory)) {
        for owner in self.retired {
            release(owner);
        }
    }
}

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
