//! Synchronous admission tests before the AP schedulers start.

use alloc::{
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::klib::observer::{
    CallOnNotify,
    Observer,
    WaitSponsor,
    registration::{
        ObserverList,
        RegistrationError,
    },
    waiter_budget::{
        self,
        DomainBudget,
    },
};

pub fn test_waiter_admission() {
    let baseline = waiter_budget::node_used();
    assert_eq!(baseline, (0, 0));
    let sponsor = WaitSponsor::new(false);
    let list = ObserverList::try_new(waiter_budget::SOURCE_LIMIT).unwrap();
    let notified = Arc::new(AtomicUsize::new(0));
    let count = notified.clone();
    let reentrant_list = list.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        // Notification must release the independent list lock first.
        assert_eq!(reentrant_list.registered(), 0);
        count.fetch_add(1, Ordering::Relaxed);
    });
    let mut registrations = Vec::new();
    for _ in 0..waiter_budget::SOURCE_LIMIT {
        let registration = sponsor.register(&list, Arc::downgrade(&observer)).unwrap();
        assert!(registration.is_owned());
        registrations.push(registration);
    }
    assert_eq!(sponsor.used(), waiter_budget::SOURCE_LIMIT);
    assert!(matches!(
        sponsor.register(&list, Arc::downgrade(&observer)),
        Err(RegistrationError::ResourceLimit)
    ));
    assert_eq!(sponsor.used(), waiter_budget::SOURCE_LIMIT);
    drop(registrations.pop());
    assert_eq!(list.registered(), waiter_budget::SOURCE_LIMIT - 1);
    registrations.push(sponsor.register(&list, Arc::downgrade(&observer)).unwrap());
    let batch = list.drain();
    assert_eq!(list.registered(), 0);
    drop(registrations);
    assert_eq!(sponsor.used(), waiter_budget::SOURCE_LIMIT, "detached entries retain admission");
    batch.notify();
    assert_eq!(notified.load(Ordering::Relaxed), waiter_budget::SOURCE_LIMIT);
    assert_eq!(sponsor.used(), 0);
    let rearmed = sponsor.register(&list, Arc::downgrade(&observer)).unwrap();
    let closed = list.close();
    assert!(matches!(
        sponsor.register(&list, Arc::downgrade(&observer)),
        Err(RegistrationError::Closed)
    ));
    drop(rearmed);
    assert_eq!(sponsor.used(), 1);
    drop(closed);
    assert_eq!(sponsor.used(), 0);

    // Domain rejection happens before allocating another source entry.
    let mut sources = Vec::new();
    let mut registrations = Vec::new();
    for _ in 0..waiter_budget::DOMAIN_LIMIT / waiter_budget::SOURCE_LIMIT {
        let source = ObserverList::try_new(waiter_budget::SOURCE_LIMIT).unwrap();
        for _ in 0..waiter_budget::SOURCE_LIMIT {
            registrations.push(sponsor.register(&source, Arc::downgrade(&observer)).unwrap());
        }
        sources.push(source);
    }
    let extra = ObserverList::try_new(waiter_budget::SOURCE_LIMIT).unwrap();
    assert!(matches!(
        sponsor.register(&extra, Arc::downgrade(&observer)),
        Err(RegistrationError::ResourceLimit)
    ));
    assert_eq!(extra.registered(), 0);
    assert_eq!(sponsor.used(), waiter_budget::DOMAIN_LIMIT);
    drop(registrations);
    assert_eq!(sponsor.used(), 0);
    assert!(sources.iter().all(|source| source.registered() == 0));

    // Counter-only saturation isolates ordinary/node admission and rollback.
    let ordinary = DomainBudget::new(false);
    let mut charges = Vec::new();
    for _ in 0..waiter_budget::ORDINARY_LIMIT / waiter_budget::DOMAIN_LIMIT {
        let domain = DomainBudget::new(false);
        for _ in 0..waiter_budget::DOMAIN_LIMIT {
            charges.push(waiter_budget::reserve(&domain).unwrap());
        }
    }
    assert!(matches!(waiter_budget::reserve(&ordinary), Err(RegistrationError::ResourceLimit)));
    assert_eq!(ordinary.used(), 0);
    for _ in
        0..(waiter_budget::NODE_LIMIT - waiter_budget::ORDINARY_LIMIT) / waiter_budget::DOMAIN_LIMIT
    {
        let domain = DomainBudget::new(true);
        for _ in 0..waiter_budget::DOMAIN_LIMIT {
            charges.push(waiter_budget::reserve(&domain).unwrap());
        }
    }
    let platform = DomainBudget::new(true);
    assert!(matches!(waiter_budget::reserve(&platform), Err(RegistrationError::ResourceLimit)));
    assert_eq!(platform.used(), 0);
    assert_eq!(
        waiter_budget::node_used(),
        (waiter_budget::NODE_LIMIT, waiter_budget::ORDINARY_LIMIT)
    );
    drop(charges);

    // Promotion changes future charges only, never reclassifies live ones.
    let before = waiter_budget::reserve(&ordinary).unwrap();
    ordinary.mark_platform();
    let after = waiter_budget::reserve(&ordinary).unwrap();
    assert_eq!(waiter_budget::node_used(), (2, 1));
    drop(before);
    assert_eq!(waiter_budget::node_used(), (1, 0));
    drop(after);

    // A retained old-generation entry cannot release the replacement's quota.
    let handle = crate::service::loader::create_user_address_space_handle();
    let old = crate::memory::budget::waiter_sponsor(handle.id());
    let source = ObserverList::try_new(waiter_budget::SOURCE_LIMIT).unwrap();
    let token = old.register(&source, Arc::downgrade(&observer)).unwrap();
    crate::memory::budget::retire(handle);
    assert!(matches!(
        old.register(&source, Arc::downgrade(&observer)),
        Err(RegistrationError::Closed)
    ));
    crate::memory::close_user_address_space_handle(handle).unwrap();
    let replacement = crate::service::loader::create_user_address_space_handle();
    assert_eq!(handle.id(), replacement.id());
    assert_ne!(handle.generation(), replacement.generation());
    let new = crate::memory::budget::waiter_sponsor(replacement.id());
    let new_token = new.register(&source, Arc::downgrade(&observer)).unwrap();
    assert_eq!(old.used(), 1);
    assert_eq!(new.used(), 1);
    drop(token);
    assert_eq!(old.used(), 0);
    assert_eq!(new.used(), 1);
    drop(new_token);
    crate::memory::close_user_address_space_handle(replacement).unwrap();
    assert_eq!(waiter_budget::node_used(), baseline);
    crate::logln!(
        "[waiters] SUCCESS: source/domain/node bounds, owning cancellation, detached charges, \
         promotion and ASID reuse"
    );
}
