//! Complete mapping-retirement custody: backing fence, detached records/peer
//! leases, leaf removal, invalidation, scratch and ordered explicit release.
//! Abandonment retains existing owners; no retry or physical proof is added.
use super::*;

pub(crate) mod tests;

/// Drop retains complete mapping nodes/peer leases with the backing fence.
#[must_use]
pub(super) struct RetiredObjectMappings {
    pin: MappingRetirementPin,
    pages: usize,
    mappings: RetiredMappings,
    detached: bool,
}

/// This owner has no releasing Drop. An unfinished operation's pin remains in
/// the registry as quarantine; it can neither free nor rendezvous under an
/// unknown outer guard. Object IDs are monotonic, never re-resolved via ASID.
#[must_use]
pub(super) struct MappingRetirementPin {
    object: MemoryObjectId,
    retired_mapping: Option<mapping::storage::RetiredMapping>,
    // Copied under the existing publication/claim hold. Reporting must not
    // reenter the registry from a guarded rollback or retaining error path.
    diagnostic_mapping: Option<MemoryMappingState>,
}

impl Drop for MappingRetirementPin {
    fn drop(&mut self) {
        // No decrement: only explicit, consuming release after quiescence can
        // discharge this pin. Abandonment retains backing/charge in registry.
    }
}

impl MappingRetirementPin {
    pub(super) fn object_id(&self) -> MemoryObjectId {
        self.object
    }

    /// Consume the same detached node into the pin before registry unlock.
    /// The caller retains its existing serialization and completion context.
    pub(super) fn retain_detached_mapping(
        &mut self,
        mapping: Option<mapping::storage::RetiredMapping>,
    ) {
        self.retired_mapping = mapping;
    }

    pub(super) fn retired_mapping(&self) -> Option<&mapping::storage::RetiredMapping> {
        self.retired_mapping.as_ref()
    }

    pub(super) fn with_mapping(mut self, mapping: MemoryMappingState) -> Self {
        self.diagnostic_mapping = Some(mapping);
        self
    }

    pub(super) fn record_mapping_prefix(&mut self, installed_pages: usize) {
        self.diagnostic_mapping
            .as_mut()
            .expect("mapping pin lost its diagnostics")
            .installed_pages = installed_pages;
    }

    /// Explicit observation, never a Drop action. The original pin captures
    /// metadata at publication/claim; a detached rollback record supplies its
    /// later prefix. No registry, allocator or root lookup occurs. The copied
    /// state supplies no cleanup/retry authority.
    pub(super) fn report_failure(
        &self,
        asid: AddressSpaceId,
        pages: usize,
        phase: &'static str,
        error: MemoryObjectError,
        prior: Result<(), MemoryObjectError>,
    ) {
        let mapping = self
            .retired_mapping
            .as_ref()
            .filter(|mapping| mapping.value().0 == asid)
            .map(|mapping| mapping.value().1.state)
            .or(self.diagnostic_mapping.filter(|mapping| mapping.address_space.id() == asid));
        crate::early_logln!(
            "[memory mapping retained] object={} asid={} phase={} error={:?} prior={:?} pages={} \
             mapping={:?}",
            self.object,
            asid,
            phase,
            error,
            prior,
            pages,
            mapping
        );
    }

    pub(super) fn acquire(registry: &mut MemoryObjectRegistry, object: MemoryObjectId) -> Self {
        let entry = registry.objects.get_mut(&object).expect("pinning object missing");
        entry.retirement_pins =
            entry.retirement_pins.checked_add(1).expect("retirement pin overflow");
        Self {
            object,
            retired_mapping: None,
            diagnostic_mapping: None,
        }
    }

    /// The pin keeps the immutable frame list and its charge alive, including
    /// across the last DMA/copy unpin. Copy only borrowed physical identities;
    /// neither these batches nor the page-table walker own the data frames.
    /// Caller retains a live-operation lease or lifecycle/IPC serialization
    /// for the target generation.
    pub(super) fn unmap_with(
        &self,
        asid: AddressSpaceId,
        base: VAddr,
        pages: usize,
        mut unmap: impl FnMut(AddressSpaceId, VAddr, &[PAddr]) -> Result<(), MemoryObjectError>,
    ) -> Result<(), MemoryObjectError> {
        {
            let registry = MEMORY_OBJECTS.lock();
            let object = registry.objects.get(&self.object).expect("pinned object missing");
            if pages > object.frames.len() {
                return Err(MemoryObjectError::UnmapFailed);
            }
        }
        let mut error = None;
        for first in (0..pages).step_by(RETIREMENT_FRAME_BATCH) {
            let count = (pages - first).min(RETIREMENT_FRAME_BATCH);
            let mut frames = [PAddr::from(0u64); RETIREMENT_FRAME_BATCH];
            {
                let registry = MEMORY_OBJECTS.lock();
                let object = registry.objects.get(&self.object).expect("pinned object missing");
                frames[..count].copy_from_slice(&object.frames[first..first + count]);
            }
            // Never acquire ADDRESS_SPACE_TABLE with MEMORY_OBJECTS held.
            // Table walks and registry metadata have separate allocator/lock
            // dependencies; backing retention does not require nesting them.
            if let Err(detach_error) = unmap(asid, base + (first * PAGE_SIZE), &frames[..count]) {
                error.get_or_insert(detach_error);
            }
        }
        error.map_or(Ok(()), Err)
    }

    pub(super) fn release(self, closing_asid: Option<AddressSpaceId>) {
        let _ = self.release_checked(closing_asid);
    }

    pub(super) fn release_checked(
        mut self,
        closing_asid: Option<AddressSpaceId>,
    ) -> Result<(), MemoryObjectError> {
        // Keep the registry pin and original backing admission through actual
        // detached-node disposal. Only this confirmed consuming path may free it.
        if let Some(mapping) = self.retired_mapping.take() {
            mapping::storage::release(mapping);
        }
        let (backing, borrower) = {
            let mut registry = MEMORY_OBJECTS.lock();
            let object = registry.objects.get_mut(&self.object).expect("retiring object missing");
            object.retirement_pins =
                object.retirement_pins.checked_sub(1).expect("retirement pin underflow");
            let mut borrower = closing_asid.and_then(|asid| clear_borrower(object, asid));
            if let Some(borrower) = borrower.as_mut() {
                detach_borrower_authority(&mut registry, borrower.value_mut());
            }
            (take_deferred_frames_if_unpinned(&mut registry, self.object), borrower)
        };
        if let Some(borrower) = borrower {
            release_borrower(borrower);
        }
        backing.map_or(Ok(()), ChargedFrames::release)
    }
}

pub(super) enum RetiredMappings {
    All(mapping::storage::Mappings),
    One(mapping::storage::RetiredMapping),
}

impl RetiredMappings {
    fn for_each(&self, mut visit: impl FnMut(AddressSpaceId, MemoryMappingState)) {
        match self {
            Self::All(mappings) => {
                for (&asid, mapping) in mappings.iter() {
                    visit(asid, mapping.state);
                }
            }
            Self::One(node) => {
                let (asid, mapping) = node.value();
                visit(*asid, mapping.state);
            }
        }
    }

    fn release(self) {
        match self {
            Self::All(mut mappings) => {
                while let Some((&asid, _)) = mappings.first_key_value() {
                    mapping::storage::release(mappings.take(&asid).unwrap());
                }
            }
            Self::One(node) => mapping::storage::release(node),
        }
    }
}

impl RetiredObjectMappings {
    pub(super) fn new(pin: MappingRetirementPin, pages: usize, mappings: RetiredMappings) -> Self {
        Self {
            pin,
            pages,
            mappings,
            detached: false,
        }
    }

    pub(super) fn object_id(&self) -> MemoryObjectId {
        self.pin.object
    }

    /// Raw boot fixtures retain lifecycle serialization through finish.
    /// Production namespace cleanup prepares exact peer leases in admitted
    /// mapping records before moving them into the same physical receipt.
    pub(super) fn prepare(
        mut registry: lock_api::MutexGuard<
            '_,
            crate::cpu::multiprocessor::spin::mutex::MutexCore,
            MemoryObjectRegistry,
        >,
        object_id: MemoryObjectId,
        closing_asid: AddressSpaceId,
    ) -> Self {
        let pin = MappingRetirementPin::acquire(&mut registry, object_id);
        let object = registry.objects.get_mut(&object_id).expect("retiring object missing");
        let mappings = if object.owner == closing_asid {
            object.destroy_when_unpinned = true;
            RetiredMappings::All(core::mem::take(&mut object.mappings))
        } else {
            RetiredMappings::One(
                object.mappings.take(&closing_asid).expect("retiring mapping missing"),
            )
        };
        let receipt = Self {
            pin,
            pages: object.frames.len(),
            mappings,
            detached: false,
        };
        // Consume the guard at this boundary: chained calls must not extend a
        // borrowed temporary guard through detach_with's table walk.
        drop(registry);
        receipt
    }

    /// Called only after the preparation registry guard has been released.
    /// No fallible metadata allocation or registry/table nesting is needed.
    pub(super) fn detach_with(
        mut self,
        mut unmap: impl FnMut(AddressSpaceId, VAddr, &[PAddr]) -> Result<(), MemoryObjectError>,
    ) -> Self {
        self.detached = true;
        self.mappings.for_each(|asid, mapping| {
            self.detached &= self
                .pin
                .unmap_with(asid, mapping.base, mapping.installed_pages, &mut unmap)
                .is_ok();
        });
        self
    }

    pub(super) fn finish(self, closing_asid: AddressSpaceId) {
        self.finish_with(closing_asid, |asid, base, pages| {
            crate::cpu::isa::memory::tlb::try_inval_range_user(asid, base, pages).is_ok()
        });
    }

    fn finish_with(
        self,
        closing_asid: AddressSpaceId,
        invalidate: impl FnMut(AddressSpaceId, VAddr, usize) -> bool,
    ) {
        self.finish_with_scratch(closing_asid, invalidate, release_scratch);
    }

    fn finish_with_scratch(
        self,
        closing_asid: AddressSpaceId,
        invalidate: impl FnMut(AddressSpaceId, VAddr, usize) -> bool,
        release: impl FnMut(AddressSpaceId, VAddr, usize) -> Result<(), MemoryObjectError>,
    ) {
        let _ = self.complete_with_scratch(closing_asid, invalidate, release);
    }

    pub(super) fn complete_with_scratch(
        self,
        closing_asid: AddressSpaceId,
        mut invalidate: impl FnMut(AddressSpaceId, VAddr, usize) -> bool,
        mut release: impl FnMut(AddressSpaceId, VAddr, usize) -> Result<(), MemoryObjectError>,
    ) -> Result<(), MemoryObjectError> {
        let mut quiescent = true;
        self.mappings.for_each(|asid, mapping| {
            quiescent &= invalidate(asid, mapping.base, mapping.installed_pages);
        });
        if !self.detached || !quiescent {
            crate::early_logln!(
                "[memory object] quarantined object={} pages={} detached={} quiescent={}",
                self.pin.object,
                self.pages,
                self.detached,
                quiescent
            );
            return Err(MemoryObjectError::UnmapFailed);
        }
        let mut scratch_released = true;
        self.mappings.for_each(|asid, mapping| {
            if asid != closing_asid && mapping.scratch {
                scratch_released &= release(asid, mapping.base, self.pages).is_ok();
            }
        });
        if !scratch_released {
            crate::early_logln!(
                "[memory object] quarantined object={} pages={} scratch release failed",
                self.pin.object,
                self.pages
            );
            return Err(MemoryObjectError::OutOfScratch);
        }
        self.pin.release_checked(Some(closing_asid))?;
        // Only physical success permits node disposal and peer-root release.
        // Failed/abandoned receipts retain complete nodes and inline leases.
        self.mappings.release();
        Ok(())
    }
}
