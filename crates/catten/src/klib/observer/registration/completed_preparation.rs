//! Serialized completed-entry ownership using both production charge types.

use alloc::vec::Vec;

use super::*;
use crate::{
    completion::watch_budget,
    cpu::{
        isa::lp::ops::get_int_state,
        multiprocessor::interrupt_tracking::LocalInterruptMask,
    },
    klib::observer::{
        CallOnNotify,
        WaitSponsor,
        waiter_budget,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn context<C>(list: &ListRef<C>) {
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(list.state.try_lock().is_some());
}

fn ordinary<C: core::fmt::Debug>(mut reserve: impl FnMut() -> C, used: impl Fn() -> usize) {
    let list = ObserverList::try_new(32, false).unwrap();
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("unused prepared entry notified"));
    let mut preparations = Vec::with_capacity(64);
    let mut tokens = Vec::with_capacity(32);
    for _ in 0..64 {
        preparations
            .push(PreparedRegistration::try_new(Arc::downgrade(&observer), reserve()).unwrap());
    }
    for (index, entry) in preparations.drain(32..).enumerate() {
        entry.finish_with(|| {
            context(&list);
            assert_eq!(used(), 64 - index);
            assert_eq!(Arc::weak_count(&observer), 64 - index);
        });
    }
    assert_eq!(used(), 32);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for entry in &mut preparations {
            tokens.push(list.register_prepared(entry).unwrap());
            assert!(entry.0.is_none());
        }
    }
    for entry in preparations {
        entry.finish_with(|| {
            context(&list);
            assert_eq!(used(), 32, "published entry lost original charge");
        });
    }
    assert_eq!(list.registered(), 32);
    let mut full = PreparedRegistration::try_new(Arc::downgrade(&observer), reserve()).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(
            list.register_prepared(&mut full).unwrap_err(),
            RegistrationError::ResourceLimit
        );
    }
    assert_eq!(used(), 33);
    full.finish_with(|| context(&list));
    assert_eq!(used(), 32);
    // The direct adapter must explicitly finish its rejected completed entry.
    assert_eq!(
        list.register(Arc::downgrade(&observer), reserve()).unwrap_err(),
        RegistrationError::ResourceLimit
    );
    assert_eq!(used(), 32);

    let batch = list.close();
    assert_eq!(used(), 32);
    drop(batch);
    assert_eq!(used(), 0);
    assert_eq!(Arc::weak_count(&observer), 0);
    let mut closed = PreparedRegistration::try_new(Arc::downgrade(&observer), reserve()).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(list.register_prepared(&mut closed).unwrap_err(), RegistrationError::Closed);
    }
    assert_eq!(used(), 1);
    closed.finish_with(|| context(&list));
    assert_eq!(used(), 0);
    assert_eq!(
        list.register(Arc::downgrade(&observer), reserve()).unwrap_err(),
        RegistrationError::Closed
    );
    assert_eq!(used(), 0);
    drop(tokens);
    assert_eq!(list.registered(), 0);
    // This completed unused owner now holds the callback allocation's last
    // weak reference, independently of the already closed source.
    let final_entry = PreparedRegistration::try_new(Arc::downgrade(&observer), reserve()).unwrap();
    drop(observer);
    final_entry.finish_with(|| context(&list));
    assert_eq!(used(), 0);
}

#[allow(clippy::drop_non_drop)]
fn abandon<C>(charge: C, list: &ListRef<C>) {
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("abandoned prepared entry notified"));
    let entry = PreparedRegistration::try_new(Arc::downgrade(&observer), charge).unwrap();
    assert_eq!(entry.0.as_ref().unwrap().node.id, 0);
    assert!(entry.0.as_ref().unwrap().node.next.is_none());
    drop(observer);
    let mask = LocalInterruptMask::new();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _list = list.state.lock();
        drop(entry);
    }
    drop(mask);
    assert_eq!(list.registered(), 0);
}

fn guarded_abandonment() {
    let account = watch_budget::DomainBudget::new(1);
    let watch_list = ObserverList::try_new(1, false).unwrap();
    abandon(watch_budget::reserve(&account, false).unwrap(), &watch_list);
    assert_eq!(account.used(), 1);

    let sponsor = WaitSponsor::new(false);
    let waiter_list = ObserverList::try_new(1, false).unwrap();
    abandon(waiter_budget::reserve(&sponsor.0).unwrap(), &waiter_list);
    sponsor.retire();
    assert_eq!(sponsor.used(), 1);
    drop(watch_list.close());
    drop(waiter_list.close());
    drop((watch_list, waiter_list));
    assert_eq!((account.used(), sponsor.used()), (1, 1));
    crate::logln!(
        "[completed observer entry retention] heap/list-held abandonment retained one original \
         watch charge, one original waiter charge, two complete entry Boxes and two weak-only \
         callback allocations after empty sources closed; no list, token, thread or root retained"
    );
}

pub(super) fn run() {
    let irq = get_int_state();
    let watches = watch_budget::node_used();
    let waiters = waiter_budget::node_used();
    let lists = list_budget::node_used();
    let account = watch_budget::DomainBudget::new(64);
    ordinary(|| watch_budget::reserve(&account, false).unwrap(), || account.used());
    let sponsor = WaitSponsor::new(false);
    ordinary(|| waiter_budget::reserve(&sponsor.0).unwrap(), || sponsor.used());
    assert_eq!(watch_budget::node_used(), watches);
    assert_eq!(waiter_budget::node_used(), waiters);
    assert_eq!(list_budget::node_used(), lists);
    guarded_abandonment();
    assert_eq!(watch_budget::node_used(), (watches.0 + 1, watches.1 + 1));
    assert_eq!(waiter_budget::node_used(), (waiters.0 + 1, waiters.1 + 1));
    assert_eq!(list_budget::node_used(), lists);
    assert_eq!(get_int_state(), irq);
    crate::logln!(
        "[completed observer entry] both production charge types: 64 unused finishes, 64 \
         heap-held publications with post-guard empty finishes, full/closed retained rejection, \
         direct-adapter rejection, detached discard and final-weak finish passed"
    );
}
