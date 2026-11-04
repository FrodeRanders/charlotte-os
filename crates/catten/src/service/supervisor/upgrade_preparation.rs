//! Handoff storage and every active transfer stay inside one retaining parent.
use alloc::vec::Vec;
use core::{
    mem::{
        self,
        ManuallyDrop,
    },
    ops::{
        Deref,
        DerefMut,
    },
};

use super::{
    DomainLaunchTransaction,
    UpgradeSpawnError,
};
use crate::{
    klib::admission::{
        ByteAllocator,
        IpcNodes,
    },
    memory::object::{
        MemoryObjectCap,
        PreparedTransfer,
    },
};

type Allocator = ByteAllocator<IpcNodes>;
#[must_use]
pub(super) struct PreparingUpgrade(ManuallyDrop<Resources>);
pub(super) struct Resources {
    pub(super) domain: Option<DomainLaunchTransaction>,
    pub(super) moves: Vec<PreparedTransfer, Allocator>,
    pub(super) targets: Vec<MemoryObjectCap, Allocator>,
}
impl Deref for PreparingUpgrade {
    type Target = Resources;

    fn deref(&self) -> &Resources {
        &self.0
    }
}
impl DerefMut for PreparingUpgrade {
    fn deref_mut(&mut self) -> &mut Resources {
        &mut self.0
    }
}
impl PreparingUpgrade {
    pub(super) fn new(domain: DomainLaunchTransaction, platform: bool) -> Self {
        // Capture the source namespace's exact trusted class before any
        // allocation. Old/new services cannot reclassify the staging bytes;
        // original memory-object backing sponsorship remains separate.
        let allocator = Allocator::new(platform);
        Self(ManuallyDrop::new(Resources {
            domain: Some(domain),
            moves: Vec::new_in(allocator.clone()),
            targets: Vec::new_in(allocator),
        }))
    }

    pub(super) fn prepare_storage(&mut self, count: usize) -> Result<(), UpgradeSpawnError> {
        self.moves.try_reserve_exact(count).map_err(|_| UpgradeSpawnError::Allocation)?;
        self.targets.try_reserve_exact(count).map_err(|_| UpgradeSpawnError::Allocation)?;
        Ok(())
    }

    fn dispose_transfers(&mut self) {
        // Only ordinary explicit cancellation/commit finalization invokes the
        // active child destructors, after all local publication guards leave.
        while let Some(transfer) = self.moves.pop() {
            drop(transfer);
        }
        let allocator = self.moves.allocator().clone();
        drop(mem::replace(&mut self.moves, Vec::new_in(allocator)));
        let allocator = self.targets.allocator().clone();
        drop(mem::replace(&mut self.targets, Vec::new_in(allocator)));
    }

    pub(super) fn abort<T>(mut self, error: UpgradeSpawnError) -> Result<T, UpgradeSpawnError> {
        self.dispose_transfers();
        self.domain
            .take()
            .expect("upgrade domain missing")
            .cancel()
            .map_err(UpgradeSpawnError::Thread)?;
        Err(error)
    }

    pub(super) fn finish(mut self) -> DomainLaunchTransaction {
        self.dispose_transfers();
        self.domain.take().expect("upgrade domain missing")
    }
}

#[cfg(feature = "kernel_tests")]
pub(super) mod tests;
