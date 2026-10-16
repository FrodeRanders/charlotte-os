//! Serialized lazy-source owner controls; no scheduler-admitted target leaks.

use alloc::{
    sync::Arc,
    vec::Vec,
};

use super::*;
use crate::{
    cpu::{
        isa::lp::ops::get_int_state,
        multiprocessor::interrupt_tracking::LocalInterruptMask,
    },
    klib::observer::{
        CallOnNotify,
        list_budget,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn allocate(platform: bool) -> Result<List, RegistrationError> {
    ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
}

fn context(source: &WaiterSource) {
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(source.list.try_lock().is_some());
}

fn stage(
    phase: usize,
    source: &WaiterSource,
    sponsor: &WaitSponsor,
    observer: &Arc<dyn Observer>,
) -> (Preparing, Option<WaitRegistration>) {
    let mut owner = Preparing::capture(sponsor, Arc::downgrade(observer));
    if phase >= 1 {
        owner.prepare_list(source, allocate).unwrap();
    }
    if phase == 2 {
        let mut competitor = Preparing::capture(sponsor, Arc::downgrade(observer));
        competitor.prepare_list(source, allocate).unwrap();
        competitor.select(source);
        competitor.release_candidate();
        competitor.finish();
    }
    if phase >= 2 {
        owner.select(source);
        if phase == 2 {
            assert!(!core::ptr::eq(&**owner.candidate(), &**owner.selected()));
        }
    }
    if phase >= 3 {
        owner.release_candidate_with(|| context(source));
    }
    if phase >= 4 {
        owner.prepare_entry().unwrap();
    }
    let token = if phase == 5 {
        Some(owner.publish_entry().unwrap())
    } else {
        None
    };
    (owner, token)
}

fn unused() {
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("unused lazy waiter notified"));
    for phase in 0..6 {
        let source = WaiterSource::new();
        let (owner, token) = stage(phase, &source, &sponsor, &observer);
        let mut step = 0;
        owner.finish_with(|| {
            context(&source);
            let charged = phase == 5 || (phase == 4 && step < 2);
            let weak = phase == 5 || step == 0 || (phase == 4 && step < 2);
            assert_eq!(sponsor.used(), usize::from(charged));
            assert_eq!(Arc::weak_count(&observer), usize::from(weak));
            step += 1;
        });
        assert_eq!(step, 5);
        drop(token);
        assert_eq!(sponsor.used(), 0);
        assert_eq!(Arc::weak_count(&observer), 0);
        assert_eq!(source.registered(), 0);
    }
}

fn ordinary_rejection() {
    let sponsor = WaitSponsor::new(false);
    let source = WaiterSource::new();
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("rejected lazy waiter notified"));
    let result = source.register_with(Arc::downgrade(&observer), &sponsor, |platform| {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let slot = source.list.lock();
        assert!(slot.is_none() && !platform);
        assert_eq!(sponsor.used(), 0);
        assert_eq!(Arc::weak_count(&observer), 1);
        Err(RegistrationError::AllocationFailed)
    });
    assert_eq!(result.unwrap_err(), RegistrationError::AllocationFailed);
    assert_eq!(Arc::weak_count(&observer), 0);
    assert!(source.list.lock().is_none());
    // Existing source reuse must not invoke the fresh-list allocator.
    let mut initial = Preparing::capture(&sponsor, Arc::downgrade(&observer));
    initial.prepare_list(&source, allocate).unwrap();
    initial.select(&source);
    initial.release_candidate();
    initial.finish();
    let mut pressure = Vec::with_capacity(waiter_budget::DOMAIN_LIMIT);
    for _ in 0..waiter_budget::DOMAIN_LIMIT {
        pressure.push(waiter_budget::reserve(&sponsor.0).unwrap());
    }
    assert_eq!(
        source
            .register_with(Arc::downgrade(&observer), &sponsor, |_| panic!(
                "existing source allocated"
            ))
            .unwrap_err(),
        RegistrationError::ResourceLimit
    );
    assert_eq!(sponsor.used(), waiter_budget::DOMAIN_LIMIT);
    assert_eq!(source.registered(), 0);
    assert_eq!(Arc::weak_count(&observer), 0);
    drop(pressure);
    let list = source.list.lock().as_ref().unwrap().clone();
    drop(list.close());
    assert_eq!(
        source.register(Arc::downgrade(&observer), &sponsor).unwrap_err(),
        RegistrationError::Closed
    );
    assert_eq!(sponsor.used(), 0);
    assert_eq!(Arc::weak_count(&observer), 0);
    sponsor.retire();
    let empty = WaiterSource::new();
    assert_eq!(
        empty
            .register_with(Arc::downgrade(&observer), &sponsor, |_| panic!(
                "retired sponsor allocated"
            ))
            .unwrap_err(),
        RegistrationError::Closed
    );
    assert!(empty.list.lock().is_none());
    assert_eq!(Arc::weak_count(&observer), 0);
}

fn competition() {
    let sponsor = WaitSponsor::new(false);
    let source = WaiterSource::new();
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("cancelled lazy waiter notified"));
    let lists = list_budget::node_used();
    let mut owners = Vec::with_capacity(64);
    let mut tokens = Vec::with_capacity(64);
    for _ in 0..64 {
        let mut owner = Preparing::capture(&sponsor, Arc::downgrade(&observer));
        owner.prepare_list(&source, allocate).unwrap();
        owners.push(owner);
    }
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for owner in &mut owners {
            owner.select(&source);
        }
    }
    for owner in &mut owners {
        owner.release_candidate_with(|| {
            context(&source);
            assert_eq!(sponsor.used(), 0);
            assert_eq!(Arc::weak_count(&observer), 64);
        });
    }
    assert_eq!(list_budget::node_used(), (lists.0 + 1, lists.1 + 1));
    for owner in &mut owners {
        owner.prepare_entry().unwrap();
    }
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _slot = source.list.lock();
        for owner in &mut owners {
            tokens.push(owner.publish_entry().unwrap());
        }
    }
    for owner in owners {
        owner.finish_with(|| {
            context(&source);
            assert_eq!(sponsor.used(), 64);
        });
    }
    assert_eq!(source.registered(), 64);
    assert_eq!(
        source.register(Arc::downgrade(&observer), &sponsor).unwrap_err(),
        RegistrationError::ResourceLimit
    );
    assert_eq!(sponsor.used(), 64);
    drop(tokens);
    assert_eq!(sponsor.used(), 0);
    assert_eq!(Arc::weak_count(&observer), 0);
    assert_eq!(source.registered(), 0);
}

#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    let sponsor = WaitSponsor::new(false);
    for phase in 0..6 {
        let source = WaiterSource::new();
        let observer: Arc<dyn Observer> =
            CallOnNotify::new(|| panic!("abandoned lazy waiter notified"));
        let (owner, token) = stage(phase, &source, &sponsor, &observer);
        drop(observer);
        let mask = LocalInterruptMask::new();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _slot = source.list.lock();
            drop(owner);
        }
        drop(mask);
        if phase == 5 {
            assert_eq!(sponsor.used(), 2);
        }
        // A published token remains the actual entry owner despite abandoned
        // preparation. Cancel it after both guards; never fabricate readiness.
        drop(token);
        assert_eq!(sponsor.used(), usize::from(phase >= 4));
        assert_eq!(source.registered(), 0);
        drop(source);
    }
    sponsor.retire();
    assert_eq!(sponsor.used(), 1);
    let successor = WaitSponsor::new(false);
    assert_eq!(successor.used(), 0);
    crate::logln!(
        "[lazy waiter owner retention] six guarded capture/candidate/selection/release/entry/ \
         published phases retained six lists, one original waiter charge, one entry and five \
         weak-only callback allocations after source closure and sponsor retirement"
    );
}

pub(super) fn run() {
    let irq = get_int_state();
    let waiters = waiter_budget::node_used();
    let lists = list_budget::node_used();
    unused();
    ordinary_rejection();
    competition();
    assert_eq!(waiter_budget::node_used(), waiters);
    assert_eq!(list_budget::node_used(), lists);
    guarded_abandonment();
    assert_eq!(waiter_budget::node_used(), (waiters.0 + 1, waiters.1 + 1));
    assert_eq!(list_budget::node_used(), (lists.0 + 6, lists.1 + 6));
    assert_eq!(get_int_state(), irq);
    crate::logln!(
        "[lazy waiter owner] six ordered finishes, guarded allocation/retired/quota/closed/full \
         rejection, 64 competing heap-held selections, 63 losing releases before admission, 64 \
         heap/slot-held entry publications and post-guard finish/cancellation passed"
    );
}
