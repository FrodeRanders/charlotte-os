//! Allocate a complete replacement outside the table's serialization. Only
//! scalar validation, ownership moves and buffer swaps enter publication.
use core::{
    alloc::Layout,
    marker::PhantomData,
    mem::{
        self,
        ManuallyDrop,
    },
};

use super::{
    Error,
    IdTable,
    SlotState,
};

#[derive(Debug)]
pub(crate) struct GrowthRequest<T> {
    table: usize,
    capacity: usize,
    payload: PhantomData<fn() -> T>,
}

impl<T> GrowthRequest<T> {
    pub(crate) fn amount(&self) -> Result<[u64; 2], Error> {
        let bytes = Layout::array::<Option<T>>(self.capacity)
            .map_err(|_| Error::AllocationFailed)?
            .size()
            .checked_add(
                Layout::array::<SlotState>(self.capacity)
                    .map_err(|_| Error::AllocationFailed)?
                    .size(),
            )
            .and_then(|bytes| bytes.checked_add(Layout::array::<usize>(self.capacity).ok()?.size()))
            .ok_or(Error::AllocationFailed)?;
        Ok([
            u64::try_from(bytes).map_err(|_| Error::AllocationFailed)?,
            u64::try_from(self.capacity).map_err(|_| Error::AllocationFailed)?,
        ])
    }

    pub(crate) fn prepare<C>(self, charge: C) -> Result<PreparedGrowth<T, C>, Error> {
        self.prepare_with(charge, |_| Ok(()))
    }

    fn prepare_with<C>(
        self,
        charge: C,
        mut admit: impl FnMut(usize) -> Result<(), Error>,
    ) -> Result<PreparedGrowth<T, C>, Error> {
        // This ordinary local owner handles partial allocation failure before
        // publication guards exist. Field order frees backing before charge.
        let mut storage = IdTable {
            list: alloc::vec::Vec::new(),
            available_ids: alloc::vec::Vec::new(),
            slots: alloc::vec::Vec::new(),
            admission: Some(charge),
            identity: self.table,
        };
        admit(0)?;
        storage.list.try_reserve_exact(self.capacity).map_err(|_| Error::AllocationFailed)?;
        admit(1)?;
        storage.slots.try_reserve_exact(self.capacity).map_err(|_| Error::AllocationFailed)?;
        admit(2)?;
        storage
            .available_ids
            .try_reserve_exact(self.capacity)
            .map_err(|_| Error::AllocationFailed)?;
        if storage.list.capacity() != self.capacity
            || storage.slots.capacity() != self.capacity
            || storage.available_ids.capacity() != self.capacity
        {
            return Err(Error::AllocationFailed);
        }
        Ok(PreparedGrowth {
            storage: ManuallyDrop::new(storage),
            installed: false,
        })
    }
}

/// Before installation owns empty candidate backing; afterwards owns empty
/// replaced backing. Abandonment retains all buffers and original admission,
/// even under allocator/accounting/table guards. It owns no published payload.
#[must_use]
pub(crate) struct PreparedGrowth<T, C> {
    storage: ManuallyDrop<IdTable<T, C>>,
    installed: bool,
}

impl<T, C> PreparedGrowth<T, C> {
    /// Explicit, consuming, post-guard deallocation followed by original refund.
    pub(crate) fn finish(self) {
        assert!(
            self.storage.list.is_empty()
                && self.storage.slots.is_empty()
                && self.storage.available_ids.is_empty()
        );
        drop(ManuallyDrop::into_inner(self.storage));
    }
}

impl<T, C> IdTable<T, C> {
    pub(crate) fn growth_request(&self) -> Result<Option<GrowthRequest<T>>, Error> {
        if !self.available_ids.is_empty() || self.has_insertion_capacity() {
            return Ok(None);
        }
        let capacity = self
            .list
            .len()
            .checked_add(1)
            .and_then(|len| len.max(4).checked_next_power_of_two())
            .ok_or(Error::AllocationFailed)?;
        let request = GrowthRequest {
            table: self.identity,
            capacity,
            payload: PhantomData,
        };
        request.amount()?;
        Ok(Some(request))
    }

    fn has_insertion_capacity(&self) -> bool {
        self.list.len() < self.list.capacity()
            && self.slots.len() < self.slots.capacity()
            && self.available_ids.capacity() > self.list.len()
    }

    fn install_growth(&mut self, growth: &mut PreparedGrowth<T, C>) -> Result<(), Error> {
        let storage = &mut growth.storage;
        if growth.installed || storage.identity != self.identity {
            return Err(Error::AllocationFailed);
        }
        if storage.list.capacity() <= self.list.len()
            || storage.slots.capacity() <= self.slots.len()
            || storage.available_ids.capacity() <= self.list.len()
        {
            return Err(Error::CapacityChanged);
        }
        // Capacity was checked for every destination. append moves each
        // owner exactly once and leaves empty old vectors for post-guard
        // disposal. No payload, allocator or charge destructor runs here.
        storage.list.append(&mut self.list);
        storage.slots.append(&mut self.slots);
        storage.available_ids.append(&mut self.available_ids);
        mem::swap(&mut self.list, &mut storage.list);
        mem::swap(&mut self.slots, &mut storage.slots);
        mem::swap(&mut self.available_ids, &mut storage.available_ids);
        mem::swap(&mut self.admission, &mut storage.admission);
        growth.installed = true;
        Ok(())
    }

    #[cfg(any(test, feature = "kernel_tests"))]
    pub(crate) fn install_empty_growth(
        &mut self,
        growth: &mut PreparedGrowth<T, C>,
    ) -> Result<(), Error> {
        if !self.list.is_empty() || !self.slots.is_empty() || !self.available_ids.is_empty() {
            return Err(Error::AllocationFailed);
        }
        self.install_growth(growth)
    }

    /// Recheck under the caller's publication guards. A stale/foreign/undersized
    /// candidate rejects without mutation. A competitor's usable capacity or a
    /// newly freed slot wins without installing the unused candidate.
    pub(crate) fn try_insert_prepared(
        &mut self,
        element: T,
        growth: Option<&mut PreparedGrowth<T, C>>,
    ) -> Result<usize, (T, Error)> {
        let mut element = Some(element);
        self.try_insert_from(&mut element, growth)
            .map_err(|error| (element.take().expect("rejected insertion lost payload"), error))
    }

    /// Failure leaves the complete payload in its containing owner. Returning
    /// only a scalar error avoids copying large thread owners through rejection
    /// results and cleanup frames on bounded kernel/boot stacks.
    pub(crate) fn try_insert_from(
        &mut self,
        element: &mut Option<T>,
        growth: Option<&mut PreparedGrowth<T, C>>,
    ) -> Result<usize, Error> {
        assert!(element.is_some(), "prepared insertion needs payload");
        if let Some(&id) = self.available_ids.last() {
            if self.slots[id].generation == usize::MAX {
                return Err(Error::AllocationFailed);
            }
        } else if !self.has_insertion_capacity() {
            let Some(growth) = growth else {
                return Err(Error::CapacityChanged);
            };
            self.install_growth(growth)?;
        }
        let element = element.take().expect("prepared insertion lost payload");
        if let Some(id) = self.available_ids.pop() {
            let state = &mut self.slots[id];
            assert!(
                !state.retiring && !state.closing && state.leases == 0,
                "reusing retained slot"
            );
            state.generation += 1;
            self.list[id] = Some(element);
            Ok(id)
        } else {
            let id = self.list.len();
            self.list.push(Some(element));
            self.slots.push(SlotState {
                generation: 1,
                retiring: false,
                leases: 0,
                closing: false,
                cleanup_sealed: false,
            });
            Ok(id)
        }
    }
}

#[cfg(test)]
#[path = "preparation/tests.rs"]
mod tests;
