//! Original completion buffer and admission precede every fallible record phase.

use super::*;
use crate::klib::observer::{
    registration::{
        ListRef,
        ObserverList,
        RegistrationError,
    },
    waiter_budget,
};

type Waiters = ListRef<waiter_budget::Charge>;
type Admission = ChargedAllocator<budget::Charge>;

/// Partial record construction retains every field on abandonment, without
/// allocator, budget or registry work. Ordinary rejection cancels explicitly.
#[must_use]
struct Preparing(ManuallyDrop<Resources>);

struct Resources {
    operation: OperationId,
    buffer: Option<Vec<u8>>,
    platform: bool,
    charge: Option<budget::Charge>,
    allocator: Option<Admission>,
    waiters: Option<Waiters>,
}

impl Preparing {
    fn new(operation: OperationId, buffer: Option<Vec<u8>>, charge: budget::Charge) -> Self {
        Self(ManuallyDrop::new(Resources {
            operation,
            buffer,
            platform: charge.platform(),
            charge: Some(charge),
            allocator: None,
            waiters: None,
        }))
    }

    fn admit_with(
        &mut self,
        allocate: impl FnOnce(&mut Option<budget::Charge>) -> Result<Admission, core::alloc::AllocError>,
    ) -> Result<(), SubmitError> {
        assert!(self.0.allocator.is_none() && self.0.charge.is_some());
        self.0.allocator = Some(allocate(&mut self.0.charge).map_err(|_| SubmitError::WouldBlock)?);
        Ok(())
    }

    fn waiters_with(
        &mut self,
        allocate: impl FnOnce(bool) -> Result<Waiters, RegistrationError>,
    ) -> Result<(), SubmitError> {
        assert!(self.0.allocator.is_some() && self.0.charge.is_none() && self.0.waiters.is_none());
        self.0.waiters = Some(allocate(self.0.platform).map_err(|_| SubmitError::WouldBlock)?);
        Ok(())
    }

    fn complete(mut self) -> Result<CompletionRef, SubmitError> {
        assert!(self.0.charge.is_none() && self.0.allocator.is_some() && self.0.waiters.is_some());
        let allocator = self.0.allocator.take().unwrap();
        let completion = Completion {
            operation: self.0.operation,
            inner: Mutex::new(CompletionInner {
                buffer: self.0.buffer.take(),
                state: OpState::InFlight,
                worker_observation: None,
                timer_observer: None,
                timer_cancel: None,
                event_observation: None,
                callbacks: None,
            }),
            waiters: self.0.waiters.take().unwrap(),
        };
        // The shared payload helper now owns the complete record and original
        // admission through allocation/rejection. No partial fields remain here.
        allocator.try_arc(completion).map_err(|_| SubmitError::WouldBlock)
    }

    fn cancel(self) {
        self.cancel_with(|| {});
    }

    fn cancel_with(mut self, mut before_dispose: impl FnMut()) {
        before_dispose();
        drop(self.0.buffer.take());
        before_dispose();
        drop(self.0.waiters.take());
        // Both possible forms of original admission outlive all payloads.
        before_dispose();
        drop(self.0.allocator.take());
        drop(self.0.charge.take());
    }
}

pub(super) fn new(
    operation: OperationId,
    buffer: Option<Vec<u8>>,
    charge: budget::Charge,
) -> Result<CompletionRef, SubmitError> {
    new_with(operation, buffer, charge, Admission::try_new_retaining, |platform| {
        ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
    })
}

fn new_with(
    operation: OperationId,
    buffer: Option<Vec<u8>>,
    charge: budget::Charge,
    allocate_holder: impl FnOnce(
        &mut Option<budget::Charge>,
    ) -> Result<Admission, core::alloc::AllocError>,
    allocate_list: impl FnOnce(bool) -> Result<Waiters, RegistrationError>,
) -> Result<CompletionRef, SubmitError> {
    let mut preparing = Preparing::new(operation, buffer, charge);
    let result =
        preparing.admit_with(allocate_holder).and_then(|()| preparing.waiters_with(allocate_list));
    if let Err(error) = result {
        preparing.cancel();
        return Err(error);
    }
    preparing.complete()
}

mod tests;

pub(super) fn test_preparation() {
    tests::run();
}
