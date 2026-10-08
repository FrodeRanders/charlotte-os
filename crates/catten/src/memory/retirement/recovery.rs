//! Bounded custody of complete detached roots that failed final invalidation.
//! No abandoned-owner adoption, physical-release retry or fence clearing.
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use super::{
    AddressSpaceHandle,
    RetiredAddressSpace,
};
use crate::memory::{
    AddressSpace,
    Mutex,
    PAddr,
    physical,
};

const CAPACITY: usize = 8;
const MAX_ATTEMPTS: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Ticket {
    index: usize,
    serial: u64,
}
impl Ticket {
    pub(crate) fn index(self) -> usize {
        self.index
    }

    pub(crate) fn serial(self) -> u64 {
        self.serial
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum State {
    AwaitingRetry,
    Running,
    RetryLimit,
    Recovered,
    Quarantined,
    Abandoned,
}

/// Public diagnostic copies never convey root or retry ownership.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Status {
    pub(crate) ticket: Ticket,
    pub(crate) root: AddressSpaceHandle,
    pub(crate) state: State,
    pub(crate) attempts: u32,
    pub(crate) heap_pages: u64,
    pub(crate) image_pages: u64,
    pub(crate) table_pages: u64,
    pub(crate) rejected_frames: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct Snapshot {
    pub(crate) entries: [Option<Status>; CAPACITY],
    pub(crate) rejected_admissions: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum RetryError {
    Masked,
    StaleTicket,
    Busy,
    RetryLimit,
    Terminal,
}

struct Slot {
    status: Option<Status>,
    owner: Option<RetiredAddressSpace>,
}
impl Slot {
    const fn new() -> Self {
        Self {
            status: None,
            owner: None,
        }
    }
}
struct Inner {
    slots: [Slot; CAPACITY],
    next_serial: u64,
    rejected_admissions: u64,
}

struct Registry {
    inner: Mutex<Inner>,
    // Stable, separate cells: an attempt borrows the registry and therefore
    // cannot outlive/move it. Drop touches only its admitted cell, never a lock.
    abandoned: [AtomicBool; CAPACITY],
}
impl Registry {
    const fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                slots: [const { Slot::new() }; CAPACITY],
                next_serial: 1,
                rejected_admissions: 0,
            }),
            abandoned: [const { AtomicBool::new(false) }; CAPACITY],
        }
    }

    #[allow(clippy::result_large_err)] // Return complete inline ownership outside the guard.
    fn retain(&self, owner: RetiredAddressSpace) -> Result<Ticket, RetiredAddressSpace> {
        let space = owner.entry.value();
        let mut status = Status {
            ticket: Ticket {
                index: 0,
                serial: 0,
            },
            root: owner.handle,
            state: State::AwaitingRetry,
            attempts: 0,
            heap_pages: space.heap_account.pages(),
            image_pages: space.image_account.pages(),
            table_pages: space.table_account.pages(),
            rejected_frames: 0,
        };
        let mut inner = self.inner.lock();
        let index = inner.slots.iter().position(|slot| {
            slot.status.is_none() || slot.status.is_some_and(|s| s.state == State::Recovered)
        });
        let Some(index) = index.filter(|_| inner.next_serial != u64::MAX) else {
            inner.rejected_admissions = inner.rejected_admissions.saturating_add(1);
            return Err(owner);
        };
        let ticket = Ticket {
            index,
            serial: inner.next_serial,
        };
        inner.next_serial += 1;
        status.ticket = ticket;
        let slot = &mut inner.slots[index];
        assert!(slot.owner.is_none());
        self.abandoned[index].store(false, Ordering::Release);
        slot.status = Some(status);
        slot.owner = Some(owner);
        Ok(ticket)
    }

    fn snapshot(&self) -> Snapshot {
        let inner = self.inner.lock();
        let mut entries = [None; CAPACITY];
        for (index, slot) in inner.slots.iter().enumerate() {
            entries[index] = slot.status.map(|mut status| {
                if self.abandoned[index].load(Ordering::Acquire) {
                    status.state = State::Abandoned;
                } else if status.state == State::AwaitingRetry && status.attempts >= MAX_ATTEMPTS {
                    status.state = State::RetryLimit;
                }
                status
            });
        }
        Snapshot {
            entries,
            rejected_admissions: inner.rejected_admissions,
        }
    }

    fn claim(&self, ticket: Ticket) -> Result<Attempt<'_>, RetryError> {
        let mut inner = self.inner.lock();
        let slot = inner.slots.get_mut(ticket.index).ok_or(RetryError::StaleTicket)?;
        let status =
            slot.status.as_mut().filter(|s| s.ticket == ticket).ok_or(RetryError::StaleTicket)?;
        if self.abandoned[ticket.index].load(Ordering::Acquire) {
            return Err(RetryError::Terminal);
        }
        match status.state {
            State::Running => return Err(RetryError::Busy),
            State::AwaitingRetry if status.attempts < MAX_ATTEMPTS => {}
            State::AwaitingRetry => return Err(RetryError::RetryLimit),
            _ => return Err(RetryError::Terminal),
        }
        status.attempts += 1;
        status.state = State::Running;
        let owner = slot.owner.take().expect("retained root receipt lost");
        Ok(Attempt {
            registry: self,
            ticket,
            owner: Some(owner),
            finished: false,
        })
    }

    fn retry_with(
        &self,
        ticket: Ticket,
        invalidate: impl FnOnce(&AddressSpace, AddressSpaceHandle) -> bool,
        deallocate: &mut dyn FnMut(PAddr) -> Result<(), physical::Error>,
    ) -> Result<Status, RetryError> {
        let mut attempt = self.claim(ticket)?;
        let owner = attempt.owner.take().unwrap();
        // The registry guard is gone. No controller lock survives callbacks,
        // hardware maintenance, account destruction or physical release.
        let outcome = owner.release_retry_with_physical(invalidate, deallocate);
        let mut inner = self.inner.lock();
        let slot = &mut inner.slots[ticket.index];
        let status = slot.status.as_mut().expect("claimed recovery status lost");
        assert_eq!(status.ticket, ticket);
        assert_eq!(status.state, State::Running);
        // Disarm before publishing any reusable completion slot. A later Drop
        // must never mark a successor's cell abandoned after slot reuse.
        attempt.finished = true;
        match outcome {
            Ok(rejected_frames) => {
                status.rejected_frames = rejected_frames;
                status.state = if rejected_frames == 0 {
                    State::Recovered
                } else {
                    State::Quarantined
                };
            }
            Err(owner) => {
                status.state = State::AwaitingRetry;
                slot.owner = Some(owner);
            }
        }
        let mut result = *status;
        if result.state == State::AwaitingRetry && result.attempts >= MAX_ATTEMPTS {
            result.state = State::RetryLimit;
        }
        Ok(result)
    }
}

struct Attempt<'a> {
    registry: &'a Registry,
    ticket: Ticket,
    owner: Option<RetiredAddressSpace>,
    finished: bool,
}
impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.registry.abandoned[self.ticket.index].store(true, Ordering::Release);
        }
        // RetiredEntry's Drop quarantines the complete/disarmed payload; no
        // physical release, slot completion, logging or locking in this Drop.
    }
}

static ROOT_RECOVERY: Registry = Registry::new();

#[allow(clippy::result_large_err)] // Caller retains ownership on bounded admission rejection.
pub(super) fn retain(owner: RetiredAddressSpace) -> Result<Ticket, RetiredAddressSpace> {
    ROOT_RECOVERY.retain(owner)
}

pub(crate) fn snapshot() -> Snapshot {
    ROOT_RECOVERY.snapshot()
}

/// Aggregate diagnostics only, bounded by the fixed receipt slots. Ordinary
/// domain snapshots expose none of the global recovery state.
pub(crate) fn status_words(observer: bool) -> [u64; 8] {
    if !observer {
        return [0; 8];
    }
    let snapshot = snapshot();
    let mut words = [0; 8];
    for entry in snapshot.entries.into_iter().flatten() {
        let index = match entry.state {
            State::AwaitingRetry => 0,
            State::Running => 1,
            State::RetryLimit => 2,
            State::Recovered => 3,
            State::Quarantined => 4,
            State::Abandoned => 5,
        };
        words[index] += 1;
    }
    words[6] = snapshot.rejected_admissions;
    words[7] = CAPACITY as u64;
    words
}

/// Trusted kernel controller boundary. A status ticket conveys no userspace
/// authority; no syscall, automatic worker or authenticated management adapter
/// is added here. Call only outside unrelated masking/subsystem guards.
pub(crate) fn retry(ticket: Ticket) -> Result<Status, RetryError> {
    if !crate::cpu::isa::lp::ops::get_int_state() {
        return Err(RetryError::Masked);
    }
    ROOT_RECOVERY.retry_with(ticket, super::invalidate, &mut |frame| {
        crate::memory::PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame)
    })
}

pub(crate) mod tests;
