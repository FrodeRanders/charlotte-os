//! Original source and reservation remain owned through event construction.
//! Allocation errors retain parent admission until explicit payload disposal.

use core::{
    mem::ManuallyDrop,
    sync::atomic::{
        AtomicBool,
        AtomicU32,
    },
};

use super::{
    CancellationRef,
    EventAdmission,
    PreparedEvent,
    TimerEvent,
    TimerEventCancelHandle,
    TimerEventCancellation,
    Timestamp,
    budget,
};

#[must_use]
struct PreparingEvent(ManuallyDrop<Resources>);

struct Resources {
    event: Option<TimerEvent>,
    charge: Option<budget::Charge>,
}

impl PreparingEvent {
    fn new(event: TimerEvent) -> Self {
        assert!(event.key.is_none() && event._charge.is_none() && event.cancellation.is_none());
        Self(ManuallyDrop::new(Resources {
            event: Some(event),
            charge: None,
        }))
    }

    fn reserve(&mut self, sponsor: &budget::SchedulerSponsor) -> Result<(), ()> {
        assert!(self.0.charge.is_none() && self.event()._charge.is_none());
        self.0.charge = Some(sponsor.reserve()?);
        Ok(())
    }

    fn event(&self) -> &TimerEvent {
        self.0.event.as_ref().unwrap()
    }

    fn admit(&mut self) -> Result<(), ()> {
        assert!(self.event()._charge.is_none());
        let admission = EventAdmission::try_new_retaining(&mut self.0.charge).map_err(|_| ())?;
        self.0.event.as_mut().unwrap()._charge = Some(admission);
        Ok(())
    }

    fn cancellation(&mut self) -> Result<(), ()> {
        self.cancellation_with(|admission| {
            admission
                .clone()
                .try_arc(TimerEventCancellation {
                    owner_lp: AtomicU32::new(crate::cpu::isa::lp::ops::get_lp_id()),
                    cancelled: AtomicBool::new(false),
                })
                .map_err(|_| ())
        })
    }

    fn cancellation_with(
        &mut self,
        allocate: impl FnOnce(&EventAdmission) -> Result<CancellationRef, ()>,
    ) -> Result<(), ()> {
        assert!(self.event().cancellation.is_none());
        // This is a borrow of the same admission, not an owning allocator
        // argument. Rejected cancellation allocation cannot refund the event.
        let state = allocate(self.event()._charge.as_ref().unwrap())?;
        self.0.event.as_mut().unwrap().cancellation = Some(state);
        Ok(())
    }

    fn complete(self) -> TimerEvent {
        assert!(self.0.charge.is_none() && self.event()._charge.is_some());
        ManuallyDrop::into_inner(self.0).event.unwrap()
    }

    fn cancel(self) {
        self.cancel_with(|| {});
    }

    fn cancel_with(self, mut before_dispose: impl FnMut()) {
        let Resources {
            event,
            charge,
        } = ManuallyDrop::into_inner(self.0);
        // Uninstalled admission outlives the entire original source/payload.
        // Installed admission already follows the event's existing field order.
        before_dispose();
        drop(event);
        before_dispose();
        drop(charge);
    }
}

pub(super) fn sleep(
    event: TimerEvent,
    sponsor: &budget::SchedulerSponsor,
) -> Result<PreparedEvent, ()> {
    let mut preparing = PreparingEvent::new(event);
    let result = preparing.reserve(sponsor).and_then(|()| preparing.admit());
    if let Err(error) = result {
        preparing.cancel();
        return Err(error);
    }
    PreparedEvent::new(preparing.complete())
}

pub(super) fn cancellable(
    charge: budget::Charge,
    deadline: impl FnOnce() -> Timestamp,
) -> Result<(TimerEvent, TimerEventCancelHandle), ()> {
    let mut preparing = PreparingEvent::new(TimerEvent::from(0));
    preparing.0.charge = Some(charge);
    if let Err(error) = preparing.admit().and_then(|()| preparing.cancellation()) {
        preparing.cancel();
        return Err(error);
    }
    // Preserve relative constructor timing: evaluate once after allocation.
    // Absolute deadlines are supplied unchanged by charged_at's closure.
    preparing.0.event.as_mut().unwrap().deadline = deadline();
    let handle = TimerEventCancelHandle {
        state: ManuallyDrop::new(preparing.event().cancellation.as_ref().unwrap().clone()),
    };
    Ok((preparing.complete(), handle))
}

mod tests;

pub(super) fn test_preparation() {
    tests::run();
}
