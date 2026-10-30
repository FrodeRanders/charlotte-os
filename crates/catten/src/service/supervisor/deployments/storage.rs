//! Fixed deployment bookkeeping. Preparation and publication share one slot;
//! all mutation requires the containing registry's exclusive serialization.

use core::{
    mem::ManuallyDrop,
    ops::{
        Index,
        IndexMut,
    },
};

enum Slot<T> {
    Empty,
    Preparing {
        principal: u64,
        serial: u64,
    },
    Published {
        principal: u64,
        value: T,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    InvalidPrincipal,
    Duplicate,
    Full,
    Exhausted,
}

/// Private, consuming proof of one exact preparation. Abandonment retains its
/// slot. It neither releases a domain nor authorizes retry of accepted work.
#[must_use]
#[derive(Debug)]
pub(crate) struct Reservation {
    index: usize,
    principal: u64,
    serial: u64,
}

impl Reservation {
    pub(crate) fn principal(&self) -> u64 {
        self.principal
    }
}

/// Abandonment retains every published payload; explicit removal transfers it
/// to the caller for disposal after registry guards leave.
pub(crate) struct Registry<T, const N: usize> {
    slots: ManuallyDrop<[Slot<T>; N]>,
    next_serial: u64,
}

impl<T, const N: usize> Registry<T, N> {
    pub(crate) const fn new() -> Self {
        Self {
            slots: ManuallyDrop::new([const { Slot::Empty }; N]),
            next_serial: 1,
        }
    }

    pub(crate) fn reserve(&mut self, principal: u64) -> Result<Reservation, Error> {
        if principal == 0 {
            return Err(Error::InvalidPrincipal);
        }
        if self.slots.iter().any(|slot| match slot {
            Slot::Preparing {
                principal: key,
                ..
            }
            | Slot::Published {
                principal: key,
                ..
            } => *key == principal,
            Slot::Empty => false,
        }) {
            return Err(Error::Duplicate);
        }
        let index =
            self.slots.iter().position(|slot| matches!(slot, Slot::Empty)).ok_or(Error::Full)?;
        let next = self.next_serial.checked_add(1).ok_or(Error::Exhausted)?;
        let serial = self.next_serial;
        self.next_serial = next;
        self.slots[index] = Slot::Preparing {
            principal,
            serial,
        };
        Ok(Reservation {
            index,
            principal,
            serial,
        })
    }

    fn matches(&self, reservation: &Reservation) -> bool {
        matches!(self.slots.get(reservation.index), Some(Slot::Preparing { principal, serial })
            if *principal == reservation.principal && *serial == reservation.serial)
    }

    pub(crate) fn cancel(&mut self, reservation: Reservation) {
        assert!(self.matches(&reservation), "deployment preparation identity changed");
        self.slots[reservation.index] = Slot::Empty;
    }

    pub(crate) fn publish(&mut self, reservation: Reservation, value: T) {
        assert!(self.matches(&reservation), "deployment publication identity changed");
        self.slots[reservation.index] = Slot::Published {
            principal: reservation.principal,
            value,
        };
    }

    pub(crate) fn preparing(&self, principal: u64) -> bool {
        self.slots
            .iter()
            .any(|slot| matches!(slot, Slot::Preparing { principal: key, .. } if *key == principal))
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        self.slots.iter().filter_map(|slot| match slot {
            Slot::Published {
                value,
                ..
            } => Some(value),
            _ => None,
        })
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots.iter_mut().filter_map(|slot| match slot {
            Slot::Published {
                value,
                ..
            } => Some(value),
            _ => None,
        })
    }

    /// Return the stable slot index, including holes and preparing entries.
    pub(crate) fn position(&self, mut predicate: impl FnMut(&T) -> bool) -> Option<usize> {
        self.slots.iter().position(|slot| match slot {
            Slot::Published {
                value,
                ..
            } => predicate(value),
            _ => false,
        })
    }

    pub(crate) fn remove(&mut self, index: usize) -> T {
        assert!(matches!(self.slots[index], Slot::Published { .. }));
        let Slot::Published {
            value,
            ..
        } = core::mem::replace(&mut self.slots[index], Slot::Empty)
        else {
            unreachable!()
        };
        value
    }

    /// Trusted serialized fixtures use the same reservation/publication path.
    pub(crate) fn insert(&mut self, principal: u64, value: T) {
        let reservation = self.reserve(principal).expect("fixture deployment admission failed");
        self.publish(reservation, value);
    }

    pub(crate) fn len(&self) -> usize {
        self.iter().count()
    }
}

impl<T, const N: usize> Index<usize> for Registry<T, N> {
    type Output = T;

    fn index(&self, index: usize) -> &T {
        match &self.slots[index] {
            Slot::Published {
                value,
                ..
            } => value,
            _ => panic!("deployment not published"),
        }
    }
}

impl<T, const N: usize> IndexMut<usize> for Registry<T, N> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        match &mut self.slots[index] {
            Slot::Published {
                value,
                ..
            } => value,
            _ => panic!("deployment not published"),
        }
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
