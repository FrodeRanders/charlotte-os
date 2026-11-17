//! One outstanding RNG descriptor, with terminal ownership after uncertainty.

use core::{
    mem::ManuallyDrop,
    num::NonZeroUsize,
    sync::atomic::{
        Ordering,
        fence,
    },
};

/// The split-ring layout and adapter use this same compile-time queue size.
pub const QUEUE_SIZE: u16 = 8;
const _: () = assert!(QUEUE_SIZE.is_power_of_two());

/// Hardware status has no interrupt notification in this adapter. This is a
/// per-descriptor iteration bound, not a wall-clock deadline or retry budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    completion_polls: NonZeroUsize,
}

impl Limits {
    pub const DEFAULT: Self = Self {
        completion_polls: NonZeroUsize::new(1_000_000).unwrap(),
    };

    pub const fn new(completion_polls: usize) -> Option<Self> {
        match NonZeroUsize::new(completion_polls) {
            Some(completion_polls) => Some(Self {
                completion_polls,
            }),
            None => None,
        }
    }

    pub const fn completion_polls(self) -> usize {
        self.completion_polls.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Used {
    pub descriptor: u32,
    pub length: u32,
}

/// The concrete adapter owns MMIO, queue/bounce CPU mappings, DMA mappings and
/// their original capabilities/charges together. No caller buffer is DMA-mapped.
pub trait Backend {
    type Error;
    fn prepare(&mut self, slot: usize, length: u32) -> Result<(), Self::Error>;
    fn publish(&mut self, next: u16) -> Result<(), Self::Error>;
    fn notify(&mut self);
    fn used_index(&self) -> Result<u16, Self::Error>;
    fn used_entry(&self, slot: usize) -> Result<Used, Self::Error>;
    fn copy_entropy(&self, destination: &mut [u8]) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failure {
    Interrupted,
    InterruptedAfterCompletion,
    Prepare,
    Publish,
    UsedIndex,
    UnexpectedIndex {
        expected: u16,
        observed: u16,
    },
    Timeout,
    UsedEntry,
    UnexpectedDescriptor(u32),
    InvalidUsedLength {
        requested: u32,
        observed: u32,
    },
    CopyAfterCompletion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidLength,
    Retained(Failure),
}

#[derive(Clone, Copy)]
enum State {
    Idle(u16),
    Retained(Failure),
}

/// Captured descriptor context retained with the complete backend. These
/// diagnostics are not a recovery ticket or global hardware-quiescence proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Attempt {
    pub previous_index: u16,
    pub expected_index: u16,
    pub slot: usize,
    pub requested: u32,
    pub copied_prefix: usize,
    /// Present only after exact index, descriptor and length validation.
    pub checked_completion: Option<Used>,
}

/// Complete persistent device owner. There is no reset, extraction or retry API
/// for a retained backend. Its fallback does not run any backend field Drop.
#[must_use]
pub struct Device<B> {
    backend: ManuallyDrop<B>,
    limits: Limits,
    state: State,
    attempt: Option<Attempt>,
}

impl<B: Backend> Device<B> {
    /// Adopt one initialized queue whose available/used indices are both zero
    /// and to which no descriptor has yet been submitted.
    pub fn new(backend: B, limits: Limits) -> Self {
        Self {
            backend: ManuallyDrop::new(backend),
            limits,
            state: State::Idle(0),
            attempt: None,
        }
    }

    pub fn retained_failure(&self) -> Option<Failure> {
        match self.state {
            State::Idle(_) => None,
            State::Retained(reason) => Some(reason),
        }
    }

    pub fn retained_attempt(&self) -> Option<&Attempt> {
        self.retained_failure().and(self.attempt.as_ref())
    }

    fn retain<T>(&mut self, failure: Failure) -> Result<T, Error> {
        self.state = State::Retained(failure);
        Err(Error::Retained(failure))
    }

    /// Success permits use of the complete output. Rejection may leave a
    /// partial prefix; no byte count is published by the server on failure.
    pub fn fill(&mut self, destination: &mut [u8]) -> Result<(), Error> {
        let mut position = match self.state {
            State::Idle(position) => position,
            State::Retained(reason) => return Err(Error::Retained(reason)),
        };
        if destination.is_empty() || destination.len() > crate::entropy::MAX_REQUEST {
            return Err(Error::InvalidLength);
        }
        let mut filled = 0;
        while filled < destination.len() {
            let requested = (destination.len() - filled) as u32;
            let next = position.wrapping_add(1);
            let slot = position as usize % QUEUE_SIZE as usize;
            self.attempt = Some(Attempt {
                previous_index: position,
                expected_index: next,
                slot,
                requested,
                copied_prefix: filled,
                checked_completion: None,
            });
            // Fence before the first descriptor/ring mutation, including an
            // unwind or interruption anywhere inside a backend callback.
            self.state = State::Retained(Failure::Interrupted);
            if self.backend.prepare(slot, requested).is_err() {
                return self.retain(Failure::Prepare);
            }
            fence(Ordering::Release);
            if self.backend.publish(next).is_err() {
                return self.retain(Failure::Publish);
            }
            self.backend.notify();
            let mut complete = false;
            for _ in 0..self.limits.completion_polls() {
                let observed = match self.backend.used_index() {
                    Ok(index) => index,
                    Err(_) => return self.retain(Failure::UsedIndex),
                };
                if observed == next {
                    complete = true;
                    break;
                }
                if observed != position {
                    return self.retain(Failure::UnexpectedIndex {
                        expected: next,
                        observed,
                    });
                }
            }
            if !complete {
                return self.retain(Failure::Timeout);
            }
            fence(Ordering::Acquire);
            let used = match self.backend.used_entry(slot) {
                Ok(used) => used,
                Err(_) => return self.retain(Failure::UsedEntry),
            };
            if used.descriptor != 0 {
                return self.retain(Failure::UnexpectedDescriptor(used.descriptor));
            }
            if used.length == 0 || used.length > requested {
                return self.retain(Failure::InvalidUsedLength {
                    requested,
                    observed: used.length,
                });
            }
            let end = filled + used.length as usize;
            self.attempt.as_mut().expect("active descriptor").checked_completion = Some(used);
            self.state = State::Retained(Failure::InterruptedAfterCompletion);
            if self.backend.copy_entropy(&mut destination[filled..end]).is_err() {
                return self.retain(Failure::CopyAfterCompletion);
            }
            filled = end;
            position = next;
            self.state = State::Idle(position);
            self.attempt = None;
        }
        Ok(())
    }
}

impl<B> Drop for Device<B> {
    fn drop(&mut self) {
        if matches!(self.state, State::Idle(_)) {
            // SAFETY: this initialized backend is destroyed once, only when
            // every submitted descriptor has a checked completion. Retained
            // state instead preserves every backend field without cleanup.
            unsafe { ManuallyDrop::drop(&mut self.backend) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng_device_contract::{
        Control,
        Fake,
        Mode,
    };

    #[test]
    fn used_index_wraps_only_after_exact_completion() {
        let control = Control::new(Mode::Healthy);
        control.index.set(u16::MAX);
        let mut device = Device::new(Fake::new(&control), Limits::new(3).unwrap());
        device.state = State::Idle(u16::MAX);
        assert_eq!(device.fill(&mut [0; 1]), Ok(()));
        assert_eq!(control.index.get(), 0);
        assert_eq!(device.fill(&mut [0; 1]), Ok(()));
        assert_eq!(control.index.get(), 1);
    }

    #[test]
    fn unwind_retains_the_complete_backend_before_publication() {
        extern crate std;
        let control = Control::new(Mode::Panic);
        let mut device = Device::new(Fake::new(&control), Limits::new(3).unwrap());
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.fill(&mut [0; 1])))
                .is_err()
        );
        assert_eq!(device.retained_failure(), Some(Failure::Interrupted));
        drop(device);
        assert_eq!(control.calls.get().backend_drops, 0);
        assert_eq!(control.calls.get().field_drops, 0);
    }

    #[test]
    fn unwind_after_completion_preserves_the_distinct_retention_stage() {
        extern crate std;
        let control = Control::new(Mode::CopyPanic);
        let mut device = Device::new(Fake::new(&control), Limits::new(3).unwrap());
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.fill(&mut [0; 1])))
                .is_err()
        );
        assert_eq!(device.retained_failure(), Some(Failure::InterruptedAfterCompletion));
        drop(device);
        assert_eq!(control.calls.get().backend_drops, 0);
        assert_eq!(control.calls.get().field_drops, 0);
    }
}
