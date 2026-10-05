//! Joint ownership of a provisional user data frame and its admission charge.
//! The exclusive address-space borrow remains inside the caller's table guard.
//! No reusable-ASID lookup or allocation is needed to retain failed rollback.

use super::{
    AddressSpace,
    PAddr,
    PHYSICAL_FRAME_ALLOCATOR,
    PreparingUserFrame,
    VAddr,
    backing_budget::{
        Account,
        Kind,
        PageCharge,
    },
    linear::{
        MemoryMapping,
        PageType,
    },
    physical,
};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum BackingPreparationError {
    Admission,
    Tracking,
    Allocation,
}

#[must_use]
pub(crate) struct PreparingUserBacking<'a> {
    space: &'a mut AddressSpace,
    kind: Kind,
    frame: Option<PreparingUserFrame>,
    charge: Option<PageCharge>,
    may_be_published: bool,
}

fn account(space: &mut AddressSpace, kind: Kind) -> &mut Account {
    match kind {
        Kind::Heap => &mut space.heap_account,
        Kind::Image => &mut space.image_account,
    }
}

impl<'a> PreparingUserBacking<'a> {
    pub(crate) fn new(
        space: &'a mut AddressSpace,
        kind: Kind,
    ) -> Result<Self, BackingPreparationError> {
        Self::new_with(
            space,
            kind,
            |space| space.prepare_user_frame().is_ok(),
            PreparingUserFrame::allocate,
        )
    }

    fn new_with(
        space: &'a mut AddressSpace,
        kind: Kind,
        track: impl FnOnce(&mut AddressSpace) -> bool,
        allocate: impl FnOnce() -> Option<PreparingUserFrame>,
    ) -> Result<Self, BackingPreparationError> {
        let charge =
            account(space, kind).reserve().map_err(|_| BackingPreparationError::Admission)?;
        let mut preparation = Self {
            space,
            kind,
            frame: None,
            charge: Some(charge),
            may_be_published: false,
        };
        if !track(preparation.space) {
            return Err(BackingPreparationError::Tracking);
        }
        preparation.frame = Some(allocate().ok_or(BackingPreparationError::Allocation)?);
        // Capture both owners before initialization, including interruption of
        // the zeroing step. Fill callbacks follow the same joint ownership.
        preparation.frame.as_ref().unwrap().zero();
        Ok(preparation)
    }

    pub(crate) fn fill(&mut self, fill: impl FnOnce(&mut [u8])) {
        let hhdm: *mut u8 = self.frame.as_ref().unwrap().frame().into();
        // Exclusive fresh backing has no published leaf. The callback borrows
        // one zeroed page only; it does not acquire frame ownership.
        fill(unsafe {
            core::slice::from_raw_parts_mut(hhdm, crate::cpu::isa::memory::paging::PAGE_SIZE)
        });
    }

    /// A rejecting mapper must not publish a leaf. Successful publication is
    /// followed only by infallible transfer into the preflighted root registry
    /// and its original account. No caller-side charge commit remains.
    pub(crate) fn map_with(
        mut self,
        vaddr: VAddr,
        page_type: PageType,
        map: impl FnOnce(&mut AddressSpace, MemoryMapping) -> bool,
    ) -> Result<PAddr, ()> {
        let frame = self.frame.as_ref().unwrap().frame();
        // An interrupted mapper may already have published a leaf. Drop must
        // retain backing unless rejection confirms that no leaf was installed.
        self.may_be_published = true;
        if !map(
            self.space,
            MemoryMapping {
                vaddr,
                paddr: frame,
                page_type,
            },
        ) {
            self.may_be_published = false;
            return Err(());
        }
        account(self.space, self.kind).commit_prepared(self.charge.as_mut().unwrap());
        drop(self.charge.take());
        self.frame.as_mut().unwrap().install(self.space);
        drop(self.frame.take());
        Ok(frame)
    }

    fn rollback_with(&mut self, deallocate: impl FnOnce(PAddr) -> Result<(), physical::Error>) {
        if self.frame.is_none() {
            // No backing was allocated: ordinary reservation rollback is safe.
            drop(self.charge.take());
            return;
        }
        if self.may_be_published {
            // This owner cannot prove translation quiescence. Do not release a
            // potentially reachable leaf, even if field transfer was interrupted.
            if let Some(charge) = self.charge.take() {
                if charge.is_active() {
                    let _retained = account(self.space, self.kind).retire_provisional(charge);
                } else {
                    // Account commit completed, but the inert charge token was
                    // not removed yet. Do not count that reservation twice.
                    account(self.space, self.kind).quarantine_committed_page();
                }
            } else {
                account(self.space, self.kind).quarantine_committed_page();
            }
            self.frame.take().unwrap().quarantine();
            crate::logln!(
                "[backing preparation] unconfirmed publication retained frame and charge"
            );
            return;
        }
        let retirement =
            account(self.space, self.kind).retire_provisional(self.charge.take().unwrap());
        let preparation = self.frame.take().unwrap();
        let frame = preparation.frame();
        let released = match preparation.release_with(deallocate) {
            Ok(()) => true,
            Err(error) => {
                crate::logln!(
                    "[backing preparation] release rejected frame={:#x}: {:?}",
                    usize::from(frame),
                    error
                );
                false
            }
        };
        retirement.finish(released);
    }
}

impl Drop for PreparingUserBacking<'_> {
    fn drop(&mut self) {
        self.rollback_with(|frame| PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame));
    }
}

pub(crate) mod tests;
