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
            assert!(!state.retiring && state.leases == 0, "reusing retained slot");
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
            });
            id
        }
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
        if self.slots[element_id].leases != 0 {
            return Err(Error::Leased);
        }
        reserve(&mut self.available_ids, self.list.len()).map_err(|_| Error::AllocationFailed)
    }

    pub(crate) fn retire_element(&mut self, element_id: usize) -> Result<RetiredEntry<T>, Error> {
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
            if state.leases != 0 {
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
