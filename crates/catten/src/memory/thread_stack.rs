//! Exclusive stack admission, including preparing and retiring threads.
use super::{
    ADDRESS_SPACE_TABLE,
    AddressSpaceHandle,
    operation::AddressSpaceOperation,
};

const _: () = assert!(charlotte_launch::MAX_USER_THREADS == 64);
pub(crate) const KERNEL_STACK_PAGES: usize = 16;
mod budget;
mod stacks;
pub(crate) use stacks::{
    RetirementError,
    Stacks,
};
pub(crate) fn retirement_progress() -> [u64; 6] {
    stacks::retirement_progress()
}
pub(crate) fn test_admission() {
    stacks::tests::run();
}

#[derive(Debug)]
pub(crate) enum PreparationError {
    Physical(super::physical::Error),
    Slot,
}

/// One owner for provisional physical backing and stack/root admission.
/// Panic during mapping quarantines both; ordinary rejected mapping releases
/// them only after the walker has returned a confirmed non-publication error.
pub(crate) struct PreparingStackPage {
    frame: Option<super::PreparingUserFrame>,
    slot: Option<StackSlot>,
    mapping_started: bool,
}

impl PreparingStackPage {
    pub(crate) fn reserve(handle: AddressSpaceHandle) -> Result<Self, ()> {
        let preparation = Self::with_frame(handle, super::PreparingUserFrame::allocate)?;
        preparation.frame.as_ref().unwrap().zero();
        Ok(preparation)
    }

    pub(crate) fn with_frame(
        handle: AddressSpaceHandle,
        allocate: impl FnOnce() -> Option<super::PreparingUserFrame>,
    ) -> Result<Self, ()> {
        let slot = StackSlot::reserve(handle)?;
        let mut preparation = Self {
            frame: None,
            slot: Some(slot),
            mapping_started: false,
        };
        let Some(frame) = allocate() else {
            preparation.cancel_unpublished().map_err(|_| ())?;
            return Err(());
        };
        preparation.frame = Some(frame);
        Ok(preparation)
    }

    pub(crate) fn base(&self) -> usize {
        self.slot.as_ref().unwrap().base()
    }

    pub(crate) fn map(mut self, low: usize) -> Result<StackSlot, ()> {
        use crate::cpu::isa::interface::memory::{
            AddressSpaceInterface,
            MemoryMapping,
        };
        let handle = self.slot.as_ref().unwrap().identity();
        if !self.slot.as_ref().unwrap().contains_page(low) {
            self.cancel_unpublished().map_err(|_| ())?;
            return Err(());
        }
        let frame = self.frame.as_ref().unwrap().frame();
        self.mapping_started = true;
        let mapped = {
            let mut table = ADDRESS_SPACE_TABLE.lock();
            table.generation(handle.id()).ok() == Some(handle.generation())
                && table.get_mut(handle.id()).is_ok_and(|space| {
                    space
                        .map_page(MemoryMapping {
                            vaddr: super::VAddr::from(low),
                            paddr: frame,
                            page_type: super::linear::PageType::UserData,
                        })
                        .is_ok()
                })
        };
        if !mapped {
            self.mapping_started = false;
            self.cancel_unpublished().map_err(|_| ())?;
            return Err(());
        }
        // The published leaf is now owned by the context's stack retirement.
        self.frame.take().unwrap().quarantine();
        let mut slot = self.slot.take().unwrap();
        slot.published();
        Ok(slot)
    }
}

impl PreparingStackPage {
    /// Ordinary cancellation of definitely unpublished backing. Consumes
    /// ownership before allocator entry; rejected release retains the complete
    /// original slot/root/reservation and cannot be retried through Drop.
    pub(crate) fn cancel_unpublished(mut self) -> Result<(), PreparationError> {
        self.rollback_with(|frame| super::PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame))
    }

    fn rollback_with(
        &mut self,
        release: impl FnOnce(super::PAddr) -> Result<(), super::physical::Error>,
    ) -> Result<(), PreparationError> {
        assert!(!self.mapping_started);
        let slot = self.slot.as_mut().expect("completed stack preparation");
        assert!(!slot.reachable && !slot.uncertain);
        if let Some(frame) = self.frame.take() {
            slot.uncertain = true;
            frame.release_with(release).map_err(PreparationError::Physical)?;
            slot.uncertain = false;
        }
        self.slot.take().unwrap().cancel_unpublished().map_err(|_| PreparationError::Slot)
    }
}
impl Drop for PreparingStackPage {
    fn drop(&mut self) {
        if let Some(frame) = self.frame.take() {
            frame.quarantine();
        }
        // Implicit StackSlot/operation/reservation destruction retains their
        // original counts without acquiring any guard or running cleanup.
    }
}

/// The root lease prevents ASID reuse during preparation and physical cleanup.
/// An uncertain mapping/teardown retains both the slot and lease on Drop.
pub(crate) struct StackSlot {
    operation: Option<AddressSpaceOperation>,
    slot: usize,
    reachable: bool,
    uncertain: bool,
    user_pages: usize,
    charge: Option<budget::Reservation>,
}

impl core::fmt::Debug for StackSlot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StackSlot").field("slot", &self.slot).finish_non_exhaustive()
    }
}

impl StackSlot {
    pub(crate) fn reserve(handle: AddressSpaceHandle) -> Result<Self, ()> {
        let asid = handle.id();
        let operation = AddressSpaceOperation::acquire(handle).map_err(|_| ())?;
        let limits = super::domain_limits(asid);
        let platform = super::budget::platform_identity(asid) == Some(handle);
        let mut table = ADDRESS_SPACE_TABLE.lock();
        let Ok(space) = table.get_mut(asid) else {
            drop(table);
            let _ = operation.release();
            return Err(());
        };
        let bits = space.thread_stack_slots;
        if space.thread_admission_closed
            || bits.count_ones() as usize >= limits.max_threads
            || bits == u64::MAX
        {
            drop(table);
            let _ = operation.release();
            return Err(());
        }
        let Some(charge) =
            budget::Reservation::reserve(limits.user_stack_pages + KERNEL_STACK_PAGES, platform)
        else {
            drop(table);
            let _ = operation.release();
            return Err(());
        };
        let slot = (!bits).trailing_zeros() as usize;
        space.thread_stack_slots |= 1 << slot;
        Ok(Self {
            operation: Some(operation),
            slot,
            reachable: false,
            uncertain: false,
            user_pages: limits.user_stack_pages,
            charge: Some(charge),
        })
    }

    pub(crate) fn identity(&self) -> AddressSpaceHandle {
        self.operation.as_ref().expect("released stack slot").handle()
    }

    pub(crate) fn base(&self) -> usize {
        charlotte_launch::user_address::STACK_BASE
            + self.slot * charlotte_launch::user_address::STACK_STRIDE
    }

    pub(crate) fn published(&mut self) {
        self.reachable = true;
    }

    fn contains_page(&self, low: usize) -> bool {
        low.is_multiple_of(4096)
            && (self.base()..self.base() + self.user_pages * 4096).contains(&low)
    }

    /// Cancel ordinary unused admission, before any backing is published.
    pub(crate) fn cancel_unpublished(mut self) -> Result<(), ()> {
        assert!(!self.reachable && !self.uncertain);
        self.release()
    }

    /// Only call after all leaves are detached, invalidated and released.
    pub(crate) fn released(&mut self) -> Result<(), ()> {
        if self.uncertain {
            return Err(());
        }
        self.reachable = false;
        self.release()
    }

    fn release(&mut self) -> Result<(), ()> {
        let Some(operation) = self.operation.take() else {
            return Err(());
        };
        let handle = operation.handle();
        {
            let mut table = ADDRESS_SPACE_TABLE.lock();
            if table.generation(handle.id()).ok() != Some(handle.generation()) {
                return Err(());
            }
            let space = table.get_mut(handle.id()).map_err(|_| ())?;
            if space.thread_stack_slots & (1 << self.slot) == 0 {
                return Err(());
            }
            space.thread_stack_slots &= !(1 << self.slot);
        }
        if let Some(charge) = self.charge.take() {
            charge.refund();
        }
        operation.release().map_err(|_| ())
    }
}
// No StackSlot cleanup destructor: operation/charge tokens retain admission on
// abandonment, including an unpublished or reservation-only preparation.
