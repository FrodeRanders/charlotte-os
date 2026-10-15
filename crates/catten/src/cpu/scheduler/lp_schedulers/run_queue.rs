//! Allocation-free FIFO membership owned inline by each exact thread.
//!
//! Queue heads borrow identities, never pointers into a moving thread table.
//! All links stay in the containing Thread owner until explicit removal. The
//! caller retains its LP serialization and the exclusive entry-store borrow
//! across validation and mutation. Abandoning a queue does not clear links or
//! release thread ownership. Queue identities are never reused.

use core::{
    num::NonZeroUsize,
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

static NEXT_QUEUE: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ThreadHandle {
    pub(crate) tid: usize,
    pub(crate) generation: u64,
}

#[derive(Debug)]
pub(crate) struct QueueLink {
    owner: Option<NonZeroUsize>,
    previous: Option<ThreadHandle>,
    next: Option<ThreadHandle>,
}

impl QueueLink {
    pub(crate) const fn new() -> Self {
        Self {
            owner: None,
            previous: None,
            next: None,
        }
    }

    pub(crate) fn is_linked(&self) -> bool {
        self.owner.is_some()
    }
}

/// Lookups must qualify the complete generation, and repeated lookups under
/// one exclusive store borrow must address the same inline link. Implementors
/// do no allocation, destruction, callbacks or locking in these accessors.
pub(crate) trait Entries {
    fn link(&self, handle: ThreadHandle) -> Option<&QueueLink>;
    fn link_mut(&mut self, handle: ThreadHandle) -> Option<&mut QueueLink>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    InvalidThread,
    AlreadyLinked,
    WrongQueue,
    Corrupt,
}

#[derive(Debug)]
pub struct RunQueue {
    identity: NonZeroUsize,
    head: Option<ThreadHandle>,
    tail: Option<ThreadHandle>,
    len: usize,
}

#[derive(Clone, Copy)]
struct Position {
    previous: Option<ThreadHandle>,
    next: Option<ThreadHandle>,
}

impl RunQueue {
    pub(crate) fn new() -> Self {
        let identity = NEXT_QUEUE
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("run-queue identity exhausted");
        Self {
            identity: NonZeroUsize::new(identity).unwrap(),
            head: None,
            tail: None,
            len: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    fn owned<'a>(
        &self,
        entries: &'a impl Entries,
        key: ThreadHandle,
    ) -> Result<&'a QueueLink, Error> {
        let link = entries.link(key).ok_or(Error::InvalidThread)?;
        if link.owner != Some(self.identity) {
            return Err(Error::WrongQueue);
        }
        Ok(link)
    }

    fn check_ends(&self, entries: &impl Entries) -> Result<(), Error> {
        if self.len == 0 {
            return if self.head.is_none() && self.tail.is_none() {
                Ok(())
            } else {
                Err(Error::Corrupt)
            };
        }
        let head = self.head.ok_or(Error::Corrupt)?;
        let tail = self.tail.ok_or(Error::Corrupt)?;
        let front = self.owned(entries, head)?;
        let back = self.owned(entries, tail)?;
        if front.previous.is_some()
            || back.next.is_some()
            || (self.len == 1 && (head != tail || front.next.is_some() || back.previous.is_some()))
            || (self.len > 1 && (head == tail || front.next.is_none() || back.previous.is_none()))
        {
            return Err(Error::Corrupt);
        }
        Ok(())
    }

    fn position(&self, entries: &impl Entries, key: ThreadHandle) -> Result<Position, Error> {
        self.check_ends(entries)?;
        let link = self.owned(entries, key)?;
        if self.len == 0 {
            return Err(Error::Corrupt);
        }
        if let Some(previous) = link.previous {
            if previous == key || self.owned(entries, previous)?.next != Some(key) {
                return Err(Error::Corrupt);
            }
        } else if self.head != Some(key) {
            return Err(Error::Corrupt);
        }
        if let Some(next) = link.next {
            if next == key || self.owned(entries, next)?.previous != Some(key) {
                return Err(Error::Corrupt);
            }
        } else if self.tail != Some(key) {
            return Err(Error::Corrupt);
        }
        Ok(Position {
            previous: link.previous,
            next: link.next,
        })
    }

    pub(crate) fn push_back(
        &mut self,
        entries: &mut impl Entries,
        key: ThreadHandle,
    ) -> Result<(), Error> {
        self.check_ends(entries)?;
        let link = entries.link(key).ok_or(Error::InvalidThread)?;
        if link.is_linked() {
            return Err(Error::AlreadyLinked);
        }
        if link.previous.is_some() || link.next.is_some() || self.len == usize::MAX {
            return Err(Error::Corrupt);
        }
        self.link_validated(entries, key);
        Ok(())
    }

    /// Every rejection precedes source removal. Both neighbors and destination
    /// ends are qualified before the allocation-free, infallible scalar splice.
    pub(crate) fn transfer_to(
        &mut self,
        destination: &mut Self,
        entries: &mut impl Entries,
        key: ThreadHandle,
    ) -> Result<(), Error> {
        if self.identity == destination.identity {
            return Err(Error::WrongQueue);
        }
        let position = self.position(entries, key)?;
        destination.check_ends(entries)?;
        if destination.len == usize::MAX {
            return Err(Error::Corrupt);
        }
        self.unlink_validated(entries, key, position);
        destination.link_validated(entries, key);
        Ok(())
    }

    pub(crate) fn remove(
        &mut self,
        entries: &mut impl Entries,
        key: ThreadHandle,
    ) -> Result<(), Error> {
        let position = self.position(entries, key)?;
        self.unlink_validated(entries, key, position);
        Ok(())
    }

    pub(crate) fn pop_front(
        &mut self,
        entries: &mut impl Entries,
    ) -> Result<Option<ThreadHandle>, Error> {
        self.check_ends(entries)?;
        let Some(key) = self.head else {
            return Ok(None);
        };
        self.remove(entries, key)?;
        Ok(Some(key))
    }

    fn link_validated(&mut self, entries: &mut impl Entries, key: ThreadHandle) {
        let link = entries.link_mut(key).expect("validated run-queue entry vanished");
        link.owner = Some(self.identity);
        link.previous = self.tail;
        link.next = None;
        if let Some(tail) = self.tail {
            entries.link_mut(tail).expect("validated run-queue tail vanished").next = Some(key);
        } else {
            self.head = Some(key);
        }
        self.tail = Some(key);
        self.len += 1;
    }

    fn unlink_validated(
        &mut self,
        entries: &mut impl Entries,
        key: ThreadHandle,
        position: Position,
    ) {
        if let Some(previous) = position.previous {
            entries.link_mut(previous).expect("validated run-queue predecessor vanished").next =
                position.next;
        } else {
            self.head = position.next;
        }
        if let Some(next) = position.next {
            entries.link_mut(next).expect("validated run-queue successor vanished").previous =
                position.previous;
        } else {
            self.tail = position.previous;
        }
        *entries.link_mut(key).expect("validated run-queue entry vanished") = QueueLink::new();
        self.len -= 1;
    }

    /// Traversal is bounded by the captured length. Corrupt/stale linkage is
    /// returned once; it never follows a successor generation or clears it.
    pub(crate) fn iter<'a>(
        &'a self,
        entries: &'a impl Entries,
    ) -> impl Iterator<Item = Result<ThreadHandle, Error>> + 'a {
        let mut remaining = self.len;
        let mut current = self.head;
        let mut previous = None;
        let mut finished = false;
        let mut first = true;
        core::iter::from_fn(move || {
            if finished {
                return None;
            }
            let step = (|| {
                if first {
                    self.check_ends(entries)?;
                    first = false;
                }
                if remaining == 0 {
                    if current.is_some() {
                        return Err(Error::Corrupt);
                    }
                    return Ok(None);
                }
                let key = current.ok_or(Error::Corrupt)?;
                let link = self.owned(entries, key)?;
                if link.previous != previous {
                    return Err(Error::Corrupt);
                }
                if remaining == 1 && Some(key) != self.tail {
                    return Err(Error::Corrupt);
                }
                current = link.next;
                previous = Some(key);
                remaining -= 1;
                Ok(Some(key))
            })();
            match step {
                Ok(Some(key)) => Some(Ok(key)),
                Ok(None) => {
                    finished = true;
                    None
                }
                Err(error) => {
                    finished = true;
                    Some(Err(error))
                }
            }
        })
    }
}

#[cfg(test)]
#[path = "run_queue/tests.rs"]
mod tests;
