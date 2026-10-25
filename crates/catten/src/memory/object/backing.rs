//! Complete memory-object backing custody from charged private allocation to
//! one-shot physical release and post-completion metadata/account disposal.
//! ChargedFrames Drop remains active; extraction does not qualify outer guards.
use super::*;

mod storage_tests;
pub(super) fn test_storage() {
    storage_tests::run();
}

/// Private backing keeps its unused object node. Published retirement carries
/// the complete detached object node, including its original backing charge.
struct FrameResources {
    frames: Vec<PAddr>,
    charge: Option<crate::memory::budget::Charge>,
    storage: Option<ObjectStorage>,
    retired: Option<RetiredObject>,
    failed: bool,
}

pub(super) struct ChargedFrames(core::mem::ManuallyDrop<FrameResources>);
impl Drop for ChargedFrames {
    fn drop(&mut self) {
        self.free();
    }
}

impl ChargedFrames {
    /// Borrow private unpublished backing; this does not pin a mapped object.
    pub(super) fn prepared_frames(&self) -> &[PAddr] {
        &self.0.frames
    }

    pub(super) fn admission_active(&self) -> bool {
        self.0.charge.as_ref().unwrap().active()
    }

    pub(super) fn retired(node: RetiredObject) -> Self {
        Self(core::mem::ManuallyDrop::new(FrameResources {
            frames: Vec::new(),
            charge: None,
            storage: None,
            retired: Some(node),
            failed: false,
        }))
    }

    fn frame_count(&self) -> usize {
        self.0.retired.as_ref().map_or(self.0.frames.len(), |node| node.value().1.frames.len())
    }

    fn frames_mut(&mut self) -> &mut Vec<PAddr> {
        let fields = &mut *self.0;
        match fields.retired.as_mut() {
            Some(node) => &mut node.value_mut().1.frames,
            None => &mut fields.frames,
        }
    }

    fn free(&mut self) -> bool {
        self.free_with(|frames| {
            let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
            let mut complete = true;
            for &frame in frames {
                complete &= allocator.deallocate_frame(frame).is_ok();
            }
            complete
        })
    }

    fn free_with(&mut self, mut release: impl FnMut(&[PAddr]) -> bool) -> bool {
        if self.0.failed {
            return true;
        }
        // Fence before the first physical attempt. Interruption retains every
        // field; only confirmed completion clears the fence after disposal.
        self.0.failed = true;
        let mut failed = false;
        // Preserve sixteen-frame physical batches. Consume identities before
        // each attempt; a failed or interrupted release never retries them.
        while self.frame_count() != 0 {
            let mut frames = [PAddr::from(0u64); RETIREMENT_FRAME_BATCH];
            let mut count = 0;
            for slot in &mut frames {
                let Some(frame) = self.frames_mut().pop() else {
                    break;
                };
                *slot = frame;
                count += 1;
            }
            failed |= !release(&frames[..count]);
        }
        if failed {
            self.0.failed = true;
            // ManuallyDrop keeps private fields, or the complete published
            // node/charge, without implicit refunds or allocator cleanup.
            crate::early_logln!("WARNING: failed to free memory-object frames; charge quarantined");
            return true;
        }
        drop(self.0.storage.take());
        if let Some(node) = self.0.retired.take() {
            let mut charge = None;
            node.release_with(|(_, mut object)| {
                // The complete object retained these scalar loan records until
                // physical completion. Dispose their nodes outside the registry,
                // before the original backing charge can return.
                if let LendState::Read {
                    borrowers,
                } = &mut object.lend_state
                {
                    while let Some((&asid, _)) = borrowers.first_key_value() {
                        release_borrower(borrowers.take(&asid).unwrap());
                    }
                }
                object.retired_caps.release();
                charge = Some(object.charge);
            });
            // Physical completion, payload and actual node free precede refund.
            drop(charge);
        }
        drop(core::mem::take(&mut self.0.frames));
        drop(self.0.charge.take());
        self.0.failed = false;
        false
    }

    pub(super) fn release(mut self) -> Result<(), MemoryObjectError> {
        if self.free() {
            Err(MemoryObjectError::FrameFreeFailed)
        } else {
            Ok(())
        }
    }

    pub(super) fn into_parts(
        mut self,
    ) -> (ObjectStorage, Vec<PAddr>, crate::memory::budget::Charge) {
        assert!(self.0.retired.is_none() && !self.0.failed);
        let storage = self.0.storage.take().expect("staged object storage missing");
        let frames = core::mem::take(&mut self.0.frames);
        (storage, frames, self.0.charge.take().expect("staged charge missing"))
    }
}

/// Unmapped, authority-detached backing owns its original sponsor until explicit
/// physical release. Drop quarantines: never free under an unknown outer guard.
#[must_use]
pub(crate) struct RetiredMemory {
    backing: Option<ChargedFrames>,
    metadata: cap_storage::RetiredCapabilities,
}

impl core::fmt::Debug for RetiredMemory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RetiredMemory")
            .field("pages", &self.backing.as_ref().map_or(0, |backing| backing.frame_count()))
            .finish()
    }
}

impl Drop for RetiredMemory {
    fn drop(&mut self) {
        if let Some(backing) = self.backing.take() {
            core::mem::forget(backing);
        }
    }
}

impl RetiredMemory {
    pub(super) fn new(
        backing: Option<ChargedFrames>,
        metadata: cap_storage::RetiredCapabilities,
    ) -> Self {
        Self {
            backing,
            metadata,
        }
    }

    pub(crate) fn release(self) -> Result<(), MemoryObjectError> {
        self.release_observed(|| {})
    }

    pub(super) fn release_observed(
        mut self,
        observe: impl FnOnce(),
    ) -> Result<(), MemoryObjectError> {
        // A panicking observer still leaves the backing behind this owner.
        observe();
        self.backing.take().map_or(Ok(()), ChargedFrames::release)?;
        self.metadata.release();
        Ok(())
    }
}

impl ChargedFrames {
    pub(super) fn allocate(
        owner: crate::memory::AddressSpaceHandle,
        pages: usize,
    ) -> Result<Self, MemoryObjectError> {
        Self::allocate_with(owner, pages, || {
            ObjectStorage::try_new().map_err(|_| MemoryObjectError::ResourceLimit)
        })
    }

    fn allocate_with(
        owner: crate::memory::AddressSpaceHandle,
        pages: usize,
        allocate_storage: impl FnOnce() -> Result<ObjectStorage, MemoryObjectError>,
    ) -> Result<Self, MemoryObjectError> {
        let charge = crate::memory::budget::reserve_captured(
            owner,
            crate::memory::budget::Amount {
                pages: pages as u64,
                objects: 1,
            },
        )
        .map_err(|error| match error {
            crate::memory::budget::Error::StaleDomain => MemoryObjectError::AddressSpaceMissing,
            crate::memory::budget::Error::Limit => MemoryObjectError::ResourceLimit,
        })?;
        let mut staged = Self(core::mem::ManuallyDrop::new(FrameResources {
            frames: Vec::new(),
            charge: Some(charge),
            storage: None,
            retired: None,
            failed: false,
        }));
        staged.0.storage = Some(allocate_storage()?);
        staged
            .0
            .frames
            .try_reserve_exact(pages)
            .map_err(|_| MemoryObjectError::FrameAllocFailed)?;
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        if !crate::resource_policy::frames_available(
            allocator.free_frames() as u64,
            allocator.usable_bytes() / PAGE_SIZE as u64,
            pages as u64,
        ) {
            return Err(MemoryObjectError::ResourceLimit);
        }
        for _ in 0..pages {
            staged
                .0
                .frames
                .push(allocator.allocate_frame().map_err(|_| MemoryObjectError::FrameAllocFailed)?);
        }
        drop(allocator);
        Ok(staged)
    }
}
