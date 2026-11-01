//! Real root retirement with unpublished event/cancellation backing retained.
use super::*;
use crate::{
    klib::time::duration::ExtDuration,
    memory::{
        self,
        AddressSpace,
    },
    timers::{
        PreparedEvent,
        TimerEvent,
        account_budget,
    },
};

pub(super) fn run() {
    let before = account_budget::snapshot();
    let events = node_used();
    let root = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    let sponsor = memory::budget::timer_sponsor(root.id());
    let weak = Arc::downgrade(&sponsor.0);
    let (event, handle) =
        TimerEvent::charged(ExtDuration::from_millis(60_000), sponsor.reserve().unwrap()).unwrap();
    let prepared = PreparedEvent::new(event).unwrap();
    memory::close_user_address_space_handle(root).unwrap();
    assert!(sponsor.reserve().is_err());
    assert_eq!(account_budget::snapshot(), (before.0 + 1, before.1 + 1));
    let successor =
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    assert_eq!(root.id(), successor.id());
    assert_ne!(root.generation(), successor.generation());
    let current = memory::budget::timer_sponsor(successor.id());
    assert_eq!(current.used(), 0);
    drop(sponsor);
    prepared.cancel_unpublished();
    // The real cancellation allocation, rather than a counter-only charge,
    // still retains the event and its old account after node disposal.
    assert_eq!(node_used(), (events.0 + 1, events.1 + 1));
    assert_eq!(account_budget::snapshot(), (before.0 + 2, before.1 + 2));
    handle.release();
    assert_eq!(node_used(), events);
    assert!(weak.upgrade().is_none());
    assert_eq!(account_budget::snapshot(), (before.0 + 2, before.1 + 2));
    drop(weak);
    assert_eq!(account_budget::snapshot(), (before.0 + 1, before.1 + 1));
    assert_eq!(current.used(), 0);
    drop(current);
    memory::close_user_address_space_handle(successor).unwrap();
    assert_eq!(account_budget::snapshot(), before);
    crate::logln!(
        "[timer account lifetime] actual event/node/cancellation retention across root close and \
         exact ASID reuse; final-weak backing refund and successor independence passed"
    );
}
