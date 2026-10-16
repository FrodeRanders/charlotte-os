//! Serialized pre-AP complete waiter preparation controls.

use alloc::vec::Vec;

use super::*;
use crate::{
    cpu::{
        isa::lp::ops::get_int_state,
        multiprocessor::interrupt_tracking::LocalInterruptMask,
        scheduler::threads::MASTER_THREAD_TABLE,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn source() -> Source {
    registration::ObserverList::try_new(waiter_budget::SOURCE_LIMIT, false).unwrap()
}

fn context() {
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(MASTER_THREAD_TABLE.try_write().is_some());
}

fn ordinary_rejection() {
    let source = source();
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("rejected waiter notified"));
    for mode in 0..3 {
        let sponsor = WaitSponsor::new(false);
        let mut pressure = Vec::new();
        if mode == 0 {
            for _ in 0..waiter_budget::DOMAIN_LIMIT {
                pressure.push(waiter_budget::reserve(&sponsor.0).unwrap());
            }
        } else if mode == 1 {
            sponsor.retire();
        }
        let before = sponsor.used();
        let result = PreparedWaitRegistration::new_with(
            &sponsor,
            &source,
            Arc::downgrade(&observer),
            |owner| {
                assert_eq!(mode, 2, "admission rejection reached entry factory");
                let _heap = PRIMARY_ALLOCATOR.lock();
                let _table = MASTER_THREAD_TABLE.write();
                assert!(owner.matches(&source));
                assert!(owner.0.observer.is_some() && owner.0.charge.is_some());
                assert!(owner.0.entry.is_none());
                assert_eq!(sponsor.used(), 1);
                assert_eq!(Arc::weak_count(&observer), 1);
                Err(RegistrationError::AllocationFailed)
            },
        );
        let expected = match mode {
            0 => RegistrationError::ResourceLimit,
            1 => RegistrationError::Closed,
            _ => RegistrationError::AllocationFailed,
        };
        assert!(matches!(result, Err(error) if error == expected));
        assert_eq!(sponsor.used(), before);
        assert_eq!(Arc::weak_count(&observer), 0);
        assert_eq!(source.registered(), 0);
        drop(pressure);
        assert_eq!(sponsor.used(), 0);
    }
}

fn unused_phases() {
    let sponsor = WaitSponsor::new(false);
    let source = source();
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("unused waiter notified"));
    for phase in 0..3 {
        let mut owner =
            PreparedWaitRegistration::capture(&sponsor, &source, Arc::downgrade(&observer));
        assert!(owner.matches(&source));
        if phase >= 1 {
            owner.admit().unwrap();
        }
        if phase == 2 {
            owner.entry().unwrap();
        }
        let mut step = 0;
        owner.finish_with(|| {
            context();
            let charged = phase >= 1
                && step
                    < if phase == 2 {
                        2
                    } else {
                        3
                    };
            let weak = step == 0 || (phase == 2 && step < 2);
            assert_eq!(sponsor.used(), usize::from(charged));
            assert_eq!(Arc::weak_count(&observer), usize::from(weak));
            step += 1;
        });
        assert_eq!(step, 5);
        assert_eq!(sponsor.used(), 0);
        assert_eq!(Arc::weak_count(&observer), 0);
    }
}

fn final_source_disposal() {
    let lists = list_budget::node_used();
    let sponsor = WaitSponsor::new(false);
    let source = source();
    let account = sponsor.0.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("final weak waiter notified"));
    let owner =
        PreparedWaitRegistration::new(&sponsor, &source, Arc::downgrade(&observer)).unwrap();
    drop((source, sponsor, observer));
    let mut step = 0;
    owner.finish_with(|| {
        context();
        assert_eq!(account.used(), usize::from(step < 2));
        let retained = usize::from(step < 4);
        assert_eq!(list_budget::node_used(), (lists.0 + retained, lists.1 + retained));
        step += 1;
    });
    assert_eq!(step, 5);
    assert_eq!(account.used(), 0);
    assert_eq!(list_budget::node_used(), lists);
}

fn publication() {
    let before = waiter_budget::node_used();
    let sponsor = WaitSponsor::new(false);
    let source = source();
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("cancelled waiter notified"));
    let mut preparations = Vec::with_capacity(64);
    let mut tokens = Vec::with_capacity(64);
    for _ in 0..64 {
        preparations.push(sponsor.prepare(&source, Arc::downgrade(&observer)).unwrap());
    }
    sponsor.mark_platform();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _table = MASTER_THREAD_TABLE.write();
        for preparation in &mut preparations {
            tokens.push(preparation.publish().unwrap());
        }
    }
    for preparation in preparations {
        preparation.finish_with(|| {
            context();
            assert_eq!(sponsor.used(), 64);
            assert_eq!(waiter_budget::node_used(), (before.0 + 64, before.1 + 64));
        });
    }
    assert_eq!(source.registered(), 64);
    drop(tokens);
    assert_eq!(source.registered(), 0);
    assert_eq!(sponsor.used(), 0);
    assert_eq!(Arc::weak_count(&observer), 0);
    assert_eq!(waiter_budget::node_used(), before);
}

#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    let sponsor = WaitSponsor::new(false);
    let source = source();
    for phase in 0..3 {
        let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("abandoned waiter notified"));
        let mut owner =
            PreparedWaitRegistration::capture(&sponsor, &source, Arc::downgrade(&observer));
        assert!(owner.matches(&source));
        if phase >= 1 {
            owner.admit().unwrap();
        }
        if phase == 2 {
            owner.entry().unwrap();
        }
        drop(observer);
        let mask = LocalInterruptMask::new();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _table = MASTER_THREAD_TABLE.write();
            drop(owner);
            assert_eq!(sponsor.used(), phase);
        }
        drop(mask);
    }
    drop(source.close());
    sponsor.retire();
    assert!(matches!(waiter_budget::reserve(&sponsor.0), Err(RegistrationError::Closed)));
    assert_eq!(source.registered(), 0);
    assert_eq!(sponsor.used(), 2);
    let successor = WaitSponsor::new(false);
    assert_eq!(successor.used(), 0);
    drop((successor, source, sponsor));
    crate::logln!(
        "[waiter preparation owner retention] three guarded captured/admitted/entry phases \
         retained two original waiter charges, one closed source list, one entry and three \
         weak-only callback allocations after sponsor retirement; no target thread or root \
         retained"
    );
}

pub(super) fn run() {
    let irq = get_int_state();
    let waiters = waiter_budget::node_used();
    let lists = list_budget::node_used();
    ordinary_rejection();
    unused_phases();
    final_source_disposal();
    publication();
    assert_eq!(waiter_budget::node_used(), waiters);
    assert_eq!(list_budget::node_used(), lists);
    guarded_abandonment();
    assert_eq!(waiter_budget::node_used(), (waiters.0 + 2, waiters.1 + 2));
    assert_eq!(list_budget::node_used(), (lists.0 + 1, lists.1 + 1));
    assert_eq!(get_int_state(), irq);
    crate::logln!(
        "[waiter preparation owner] quota/retired/borrowed-entry rejection, three ordered unused \
         finishes, final-weak/source disposal, 64 heap/table-held publications and post-guard \
         finishes, original classification through promotion and actual cancellation passed"
    );
}
