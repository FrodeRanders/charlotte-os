//! Serialized boot controls; reservations here are counter-only, not backing proof.
use alloc::sync::Arc;

pub(crate) fn ledger_available() -> bool {
    LEDGER.try_lock().is_some()
}

pub(crate) fn with_ledger_locked(action: impl FnOnce()) {
    let _ledger = LEDGER.lock();
    action();
}

use super::*;
use crate::{
    klib::observer::{
        CallOnNotify,
        Observable,
        Observer,
        waiter_source::WaiterSource,
    },
    memory::{
        self,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    },
};

pub(crate) fn run() {
    super::preparation::test_preparation();
    generation_lifetime();
}

fn generation_lifetime() {
    let baseline = node_snapshot();
    let old =
        memory::register_user_address_space(memory::AddressSpace::try_new_user().unwrap()).unwrap();
    // Production lookup and counter reservation with the actual heap held:
    // the first use cannot lazily allocate a map node or either sponsor.
    let (waiters, timers, charge) = {
        let _heap = PRIMARY_ALLOCATOR.lock();
        (
            waiter_sponsor(old.id()),
            timer_sponsor(old.id()),
            reserve_captured(
                old,
                Amount {
                    pages: 2,
                    objects: 1,
                },
            )
            .unwrap(),
        )
    };
    assert_eq!(waiters.used(), 0);
    assert_eq!(timers.used(), 0);
    assert!(!waiters.list_platform().unwrap());
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("account fixture fabricated readiness"));
    let source = WaiterSource::new();
    let token = source.try_register_waiter(Arc::downgrade(&observer), &waiters).unwrap();
    let event = timers.reserve().unwrap();
    assert_eq!(waiters.used(), 1);
    assert_eq!(timers.used(), 1);
    memory::close_user_address_space_handle(old).unwrap();
    assert!(LEDGER.lock().domains.contains_key(&(old.id(), old.generation())));
    assert!(waiters.list_platform().is_err());
    assert!(timers.reserve().is_err());
    let new =
        memory::register_user_address_space(memory::AddressSpace::try_new_user().unwrap()).unwrap();
    assert_eq!(new.id(), old.id());
    assert_eq!(new.generation(), old.generation() + 1);
    let new_waiters = waiter_sponsor(new.id());
    let new_timers = timer_sponsor(new.id());
    assert_eq!(new_waiters.used(), 0);
    assert_eq!(new_timers.used(), 0);
    assert!(matches!(reserve_captured(old, Amount::default()), Err(Error::StaleDomain)));
    // The old node survives logical retirement/reuse, then detaches only on
    // its last memory charge. Old timer/waiter allocations still have owners.
    drop(charge);
    assert!(!LEDGER.lock().domains.contains_key(&(old.id(), old.generation())));
    assert_eq!(waiters.used(), 1);
    assert_eq!(timers.used(), 1);
    drop(token);
    drop(event);
    assert_eq!(waiters.used(), 0);
    assert_eq!(timers.used(), 0);
    assert_eq!(new_waiters.used(), 0);
    assert_eq!(new_timers.used(), 0);
    assert!(new_waiters.list_platform().is_ok());
    drop(
        reserve_captured(
            new,
            Amount {
                pages: 1,
                objects: 1,
            },
        )
        .unwrap(),
    );
    memory::close_user_address_space_handle(new).unwrap();
    let after = node_snapshot();
    assert_eq!(after.0.used(), baseline.0.used());
    assert_eq!(after.1.used(), baseline.1.used());
    crate::logln!(
        "[memory sponsor metadata] heap-held first lookup/reservation, exact generation reuse, \
         retired memory charge and independent waiter/timer lifetimes passed"
    );
}
