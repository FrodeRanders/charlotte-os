use alloc::vec::Vec;
use core::{
    fmt::Debug,
    mem::ManuallyDrop,
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    IdNotActive,
    AllocationFailed,
    WrongRetirement,
    Leased,
    LeaseLimit,
    WrongLease,
    Closing,
}

static TABLE_IDENTITIES: AtomicUsize = AtomicUsize::new(1);

/// A detached value still owns its table slot until explicit value release
/// followed by slot completion. Drop quarantines both, without invoking T's
/// destructor under an unknown caller guard.
#[must_use]
pub(crate) struct RetiredEntry<T> {
    value: ManuallyDrop<T>,
    slot: Option<SlotRetirement>,
}

impl<T> Drop for RetiredEntry<T> {
    fn drop(&mut self) {
        // Deliberately do not run T::drop or complete its slot. The resource
        // stays quarantined unless explicit release proved quiescence.
    }
}

impl<T> RetiredEntry<T> {
    pub(crate) fn value(&self) -> &T {
        &self.value
    }

    /// The resource-specific owner must establish quiescence first. Returning
    /// this token does not make the slot reusable until its table accepts it.
    pub(crate) fn release_value(mut self) -> SlotRetirement {
        // SAFETY: this consumes the unique owner; ManuallyDrop prevents an
        // implicit destructor during abandonment or a panicking T::drop.
        unsafe {
            ManuallyDrop::drop(&mut self.value);
        }
        self.slot.take().expect("retired entry slot missing")
    }
}

/// Linear completion token bound to one table and one slot generation.
/// An abandoned token keeps the slot unavailable; there is no restoring Drop.
#[must_use]
pub(crate) struct SlotRetirement {
    table: usize,
    id: usize,
    generation: usize,
}

/// A linear live-slot retention token, without a pointer into the moving
/// vectors. Only explicit completion in its original table releases the count.
/// Abandonment cannot make a potentially unfinished operation's slot reusable.
#[must_use]
pub(crate) struct SlotLease {
    table: usize,
    id: usize,
    generation: usize,
}

impl Drop for SlotLease {
    fn drop(&mut self) {
        // No implicit completion under an unknown guard. The live value/slot
        // remain retained, not destructively retired or returned to free IDs.
    }
}

/// Sole authority to complete a staged close. Dropping this token retains its
/// admission fence and payload; it never cancels closing or returns a slot.
#[must_use]
pub(crate) struct ClosingSlot {
    table: usize,
    id: usize,
    generation: usize,
}

impl Drop for ClosingSlot {
    fn drop(&mut self) {}
}

#[derive(Debug)]
pub struct IdTable<T> {
    list: Vec<Option<T>>,
    available_ids: Vec<usize>,
    slots: Vec<SlotState>,
    identity: usize,
}

#[derive(Debug)]
struct SlotState {
    generation: usize,
    retiring: bool,
    leases: usize,
    closing: bool,
    cleanup_sealed: bool,
}

impl<T> IdTable<T> {
    pub fn new() -> Self {
        IdTable {
            list: Vec::new(),
            available_ids: Vec::new(),
            slots: Vec::new(),
            identity: TABLE_IDENTITIES
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| value.checked_add(1))
                .expect("table identity exhausted"),
        }
    }

    pub fn add_element(&mut self, element: T) -> usize {
        if let Some(id) = self.available_ids.pop() {
            let state = &mut self.slots[id];
            assert!(
                !state.retiring && !state.closing && state.leases == 0,
                "reusing retained slot"
            );
            state.generation = state.generation.checked_add(1).expect("ID generation exhausted");
            self.list[id] = Some(element);
            id
        } else {
            let id = self.list.len();
            self.list.push(Some(element));
            // Generation zero is reserved for handles that were never
            // initialized. The first occupant of every slot is generation 1.
            // Retirement shares the existing generation metadata allocation;
            // adding a slot does not introduce a third growing state vector.
            self.slots.push(SlotState {
                generation: 1,
                retiring: false,
                leases: 0,
                closing: false,
                cleanup_sealed: false,
            });
            id
        }
    }

    /// Prepare every growing metadata vector before publishing the payload.
    pub(crate) fn try_add_element(&mut self, element: T) -> Result<usize, (T, Error)> {
        if let Some(&id) = self.available_ids.last() {
            if self.slots[id].generation == usize::MAX {
                return Err((element, Error::AllocationFailed));
            }
        } else if self.list.try_reserve(1).is_err() || self.slots.try_reserve(1).is_err() {
            return Err((element, Error::AllocationFailed));
        }
        Ok(self.add_element(element))
    }

    pub fn get(&self, element_id: usize) -> Result<&T, Error> {
        self.list.get(element_id).ok_or(Error::IdNotActive)?.as_ref().ok_or(Error::IdNotActive)
    }

    pub fn get_mut(&mut self, element_id: usize) -> Result<&mut T, Error> {
        self.list.get_mut(element_id).ok_or(Error::IdNotActive)?.as_mut().ok_or(Error::IdNotActive)
    }

    /// Return the generation of the active occupant of `element_id`.
    ///
    /// A generation changes whenever a removed slot is reused, allowing
    /// long-lived handles to distinguish the new object from its predecessor.
    pub fn generation(&self, element_id: usize) -> Result<usize, Error> {
        self.get(element_id)?;
        self.slots.get(element_id).map(|state| state.generation).ok_or(Error::IdNotActive)
    }

    pub fn take_element(&mut self, element_id: usize) -> Result<T, Error> {
        self.get(element_id)?;
        if self.slots[element_id].closing {
            return Err(Error::Closing);
        }
        if self.slots[element_id].leases != 0 {
            return Err(Error::Leased);
        }
        match self.list.get_mut(element_id).ok_or(Error::IdNotActive)?.take() {
            Some(element) => {
                self.available_ids.push(element_id);
                Ok(element)
            }
            None => Err(Error::IdNotActive),
        }
    }

    pub(crate) fn lease(
        &mut self,
        element_id: usize,
        generation: usize,
    ) -> Result<SlotLease, Error> {
        if self.generation(element_id)? != generation {
            return Err(Error::WrongLease);
        }
        let state = &mut self.slots[element_id];
        if state.closing {
            return Err(Error::Closing);
        }
        state.leases = state.leases.checked_add(1).ok_or(Error::LeaseLimit)?;
        Ok(SlotLease {
            table: self.identity,
            id: element_id,
            generation,
        })
    }

    pub(crate) fn finish_lease(&mut self, lease: SlotLease) -> Result<(), Error> {
        if lease.table != self.identity
            || self.generation(lease.id).ok() != Some(lease.generation)
            || !self.slots.get(lease.id).is_some_and(|state| !state.retiring && state.leases != 0)
        {
            return Err(Error::WrongLease);
        }
        self.slots[lease.id].leases -= 1;
        Ok(())
    }

    /// A closing owner may retain a peer while draining shared resources. This
    /// does not reopen ordinary admission. Both roots must precede the sealed
    /// backing-teardown phase; exact identity is checked under the same guard.
    pub(crate) fn lease_for_close(
        &mut self,
        owner: &ClosingSlot,
        peer: usize,
        generation: usize,
    ) -> Result<SlotLease, Error> {
        self.validate_closing(owner)?;
        if self.slots[owner.id].cleanup_sealed {
            return Err(Error::Closing);
        }
        if self.generation(peer)? != generation {
            return Err(Error::WrongLease);
        }
        let state = &mut self.slots[peer];
        if state.cleanup_sealed {
            return Err(Error::Closing);
        }
        state.leases = state.leases.checked_add(1).ok_or(Error::LeaseLimit)?;
        Ok(SlotLease {
            table: self.identity,
            id: peer,
            generation,
        })
    }

    /// No cleanup lease may enter after final root teardown begins. Existing
    /// owners must drain first; a rejected seal does not change admission.
    pub(crate) fn seal_close(&mut self, slot: &ClosingSlot) -> Result<(), Error> {
        self.validate_closing(slot)?;
        if self.slots[slot.id].leases != 0 {
            return Err(Error::Leased);
        }
        self.slots[slot.id].cleanup_sealed = true;
        Ok(())
    }

    pub(crate) fn is_closing(&self, id: usize) -> Result<bool, Error> {
        self.get(id)?;
        Ok(self.slots[id].closing)
    }

    pub(crate) fn begin_close(
        &mut self,
        id: usize,
        generation: usize,
    ) -> Result<ClosingSlot, Error> {
        self.begin_close_with(id, generation, |available, slots| {
            available.try_reserve_exact(slots - available.len()).map_err(|_| ())
        })
    }

    fn begin_close_with(
        &mut self,
        id: usize,
        generation: usize,
        reserve: impl FnOnce(&mut Vec<usize>, usize) -> Result<(), ()>,
    ) -> Result<ClosingSlot, Error> {
        if self.generation(id)? != generation {
            return Err(Error::WrongRetirement);
        }
        if self.slots[id].closing {
            return Err(Error::Closing);
        }
        // Existing leases may drain, but no irreversible admission fence is
        // published until completion metadata preparation succeeds.
        reserve(&mut self.available_ids, self.list.len()).map_err(|_| Error::AllocationFailed)?;
        self.slots[id].closing = true;
        Ok(ClosingSlot {
            table: self.identity,
            id,
            generation,
        })
    }

    pub(crate) fn prepare_closing_retirement(&mut self, slot: &ClosingSlot) -> Result<(), Error> {
        self.validate_closing(slot)?;
        if self.slots[slot.id].leases != 0 {
            return Err(Error::Leased);
        }
        // Other slots may have been added during the unlocked drain interval.
        self.available_ids
            .try_reserve_exact(self.list.len() - self.available_ids.len())
            .map_err(|_| Error::AllocationFailed)
    }

    fn validate_closing(&self, slot: &ClosingSlot) -> Result<(), Error> {
        if slot.table != self.identity
            || self.generation(slot.id).ok() != Some(slot.generation)
            || !self.slots[slot.id].closing
            || self.slots[slot.id].retiring
        {
            return Err(Error::WrongRetirement);
        }
        Ok(())
    }

    pub(crate) fn retire_closing(&mut self, slot: ClosingSlot) -> Result<RetiredEntry<T>, Error> {
        self.validate_closing(&slot)?;
        if !self.slots[slot.id].cleanup_sealed {
            return Err(Error::Closing);
        }
        if self.slots[slot.id].leases != 0 {
            return Err(Error::Leased);
        }
        if self.available_ids.capacity() < self.list.len() {
            return Err(Error::AllocationFailed); // No allocation after subsystem cleanup.
        }
        let value = self.list[slot.id].take().ok_or(Error::IdNotActive)?;
        self.slots[slot.id].retiring = true;
        Ok(RetiredEntry {
            value: ManuallyDrop::new(value),
            slot: Some(SlotRetirement {
                table: slot.table,
                id: slot.id,
                generation: slot.generation,
            }),
        })
    }

    /// Prepare free-slot storage before irreversible subsystem retirement.
    /// Reserve for every existing slot, so other detached owners can finish
    /// while this one is outside the table without allocating on completion.
    pub(crate) fn prepare_retirement(&mut self, element_id: usize) -> Result<(), Error> {
        self.prepare_retirement_with(element_id, |available, slots| {
            available.try_reserve_exact(slots - available.len()).map_err(|_| ())
        })
    }

    fn prepare_retirement_with(
        &mut self,
        element_id: usize,
        reserve: impl FnOnce(&mut Vec<usize>, usize) -> Result<(), ()>,
    ) -> Result<(), Error> {
        self.get(element_id)?;
        if self.slots[element_id].closing {
            return Err(Error::Closing);
        }
        if self.slots[element_id].leases != 0 {
            return Err(Error::Leased);
        }
        reserve(&mut self.available_ids, self.list.len()).map_err(|_| Error::AllocationFailed)
    }

    // Generic host ownership fixtures exercise immediate retirement. Published
    // production roots always own a ClosingSlot before subsystem cleanup.
    #[cfg(test)]
    fn retire_element(&mut self, element_id: usize) -> Result<RetiredEntry<T>, Error> {
        self.prepare_retirement(element_id)?;
        let value = self.list[element_id].take().ok_or(Error::IdNotActive)?;
        self.slots[element_id].retiring = true;
        Ok(RetiredEntry {
            value: ManuallyDrop::new(value),
            slot: Some(SlotRetirement {
                table: self.identity,
                id: element_id,
                generation: self.slots[element_id].generation,
            }),
        })
    }

    pub(crate) fn finish_retirement(&mut self, slot: SlotRetirement) -> Result<(), Error> {
        if slot.table != self.identity
            || !self.slots.get(slot.id).is_some_and(|state| {
                state.retiring && state.generation == slot.generation && state.leases == 0
            })
        {
            return Err(Error::WrongRetirement);
        }
        // Prepared before detachment; no allocation is allowed here. A new
        // slot can only add free IDs by another preflighted retirement (or
        // an ordinary take, whose push also grows capacity when necessary).
        if self.available_ids.len() == self.available_ids.capacity() {
            return Err(Error::AllocationFailed);
        }
        self.slots[slot.id].retiring = false;
        self.slots[slot.id].closing = false;
        self.slots[slot.id].cleanup_sealed = false;
        self.available_ids.push(slot.id);
        Ok(())
    }

    pub fn iter(&self) -> core::slice::Iter<'_, Option<T>> {
        self.list.iter()
    }

    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, Option<T>> {
        self.list.iter_mut()
    }
}

impl<T> Drop for IdTable<T> {
    fn drop(&mut self) {
        for (entry, state) in self.list.iter_mut().zip(&self.slots) {
            if state.leases != 0 || state.closing {
                // Even table destruction cannot recycle a leased payload.
                // Kernel address-space tables are static; this also protects
                // generic callers and host fixtures without a recovery bypass.
                core::mem::forget(entry.take());
            }
        }
    }
}

impl<T> Default for IdTable<T> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl<T> Send for IdTable<T> where T: Send {}
unsafe impl<T> Sync for IdTable<T> where T: Sync {}

#[cfg(test)]
#[path = "id_table/tests.rs"]
mod tests;
#[cfg(test)]
extern crate alloc;
