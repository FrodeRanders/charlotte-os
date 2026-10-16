//! Serialized containing-owner controls; no scheduler-admitted target is leaked.

use alloc::{
    sync::Arc,
    vec::Vec,
};

use super::*;
use crate::{
    cpu::{
        isa::lp::ops::get_int_state,
        multiprocessor::interrupt_tracking::LocalInterruptMask,
        scheduler::threads::MASTER_THREAD_TABLE,
    },
    klib::observer::{
        CallOnNotify,
        list_budget,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn context(source: &ExitSource) {
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(MASTER_THREAD_TABLE.try_write().is_some());
    assert!(source.list.try_lock().is_some());
}

fn candidate(platform: bool) -> Result<ExitList, RegistrationError> {
    ObserverList::try_new(watch_budget::MAX_THREAD_WATCHES, platform)
}

fn rejection_and_unused() {
    let account = watch_budget::DomainBudget::new(1);
    let source = ExitSource::new();
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("unused exit watch notified"));
    // Captured without source, captured existing source, allocated candidate
    // with borrowed entry rejection, and complete unused entry.
    for phase in 0..4 {
        let lists = list_budget::node_used();
        let existing = if phase == 1 {
            Some(candidate(false).unwrap())
        } else {
            None
        };
        let mut owner = PreparingExitWatch::capture(
            existing,
            Arc::downgrade(&observer),
            watch_budget::reserve(&account, false).unwrap(),
        );
        if phase == 0 {
            assert_eq!(
                owner.prepare_with(
                    |platform| {
                        assert!(!platform);
                        let _heap = PRIMARY_ALLOCATOR.lock();
                        let _table = MASTER_THREAD_TABLE.write();
                        let _source = source.list.lock();
                        assert_eq!(account.used(), 1);
                        // The allocator receives classification only.
                        assert_eq!(Arc::weak_count(&observer), 1);
                        Err(RegistrationError::AllocationFailed)
                    },
                    |_| panic!("rejected list reached entry preparation"),
                ),
                Err(RegistrationError::AllocationFailed)
            );
        } else if phase == 2 {
            assert_eq!(
                owner.prepare_with(candidate, |owner| {
                    let _heap = PRIMARY_ALLOCATOR.lock();
                    let _table = MASTER_THREAD_TABLE.write();
                    let _source = source.list.lock();
                    assert!(owner.0.observer.is_some() && owner.0.charge.is_some());
                    assert!(owner.0.list.is_some() && owner.0.entry.is_none());
                    assert_eq!(account.used(), 1);
                    Err(RegistrationError::AllocationFailed)
                }),
                Err(RegistrationError::AllocationFailed)
            );
        } else if phase == 3 {
            owner.prepare().unwrap();
        }
        let mut step = 0;
        owner.finish_with(|| {
            context(&source);
            let entry = phase == 3;
            assert_eq!(
                account.used(),
                usize::from(
                    step < if entry {
                        2
                    } else {
                        3
                    }
                )
            );
            assert_eq!(
                Arc::weak_count(&observer),
                usize::from(
                    step < if entry {
                        2
                    } else {
                        1
                    }
                )
            );
            let retained_list = usize::from(phase != 0);
            assert_eq!(
                list_budget::node_used(),
                (lists.0 + retained_list, lists.1 + retained_list)
            );
            step += 1;
        });
        assert_eq!(step, 4);
        assert_eq!(account.used(), 0);
        assert_eq!(Arc::weak_count(&observer), 0);
        assert_eq!(list_budget::node_used(), lists);
        assert_eq!(source.registered(), 0);
    }
    // With no strong callback reference, explicit finish releases the final
    // weak allocation outside the actual heap/table/source guards too.
    let weak = Arc::downgrade(&observer);
    drop(observer);
    let owner =
        PreparingExitWatch::new(None, weak, watch_budget::reserve(&account, false).unwrap())
            .unwrap();
    owner.finish_with(|| context(&source));
    assert_eq!(account.used(), 0);
}

fn publication() {
    let account = watch_budget::DomainBudget::new(64);
    let source = ExitSource::new();
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("cancelled exit watch notified"));
    let lists = list_budget::node_used();
    let mut preparations = Vec::with_capacity(64);
    let mut tokens = Vec::with_capacity(64);
    for _ in 0..64 {
        preparations.push(
            PreparingExitWatch::new(
                None,
                Arc::downgrade(&observer),
                watch_budget::reserve(&account, false).unwrap(),
            )
            .unwrap(),
        );
    }
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _table = MASTER_THREAD_TABLE.write();
        for preparation in &mut preparations {
            tokens.push(source.register_prepared(preparation).unwrap());
        }
    }
    assert_eq!(source.registered(), 64);
    for preparation in preparations {
        preparation.finish_with(|| {
            context(&source);
            assert_eq!(account.used(), 64);
        });
    }
    assert_eq!(list_budget::node_used(), (lists.0 + 1, lists.1 + 1));
    drop(tokens);
    assert_eq!(account.used(), 0);
    assert_eq!(Arc::weak_count(&observer), 0);
    assert_eq!(source.registered(), 0);
    drop(source);
    assert_eq!(list_budget::node_used(), lists);
}

#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    let account = watch_budget::DomainBudget::new(4);
    let mut source = ExitSource::new();
    let original = candidate(false).unwrap();
    *source.list.lock() = Some(original.clone());
    for phase in 0..4 {
        let observer: Arc<dyn Observer> =
            CallOnNotify::new(|| panic!("abandoned exit watch notified"));
        let mut owner = PreparingExitWatch::capture(
            if phase == 1 {
                source.source()
            } else {
                None
            },
            Arc::downgrade(&observer),
            watch_budget::reserve(&account, false).unwrap(),
        );
        if phase == 2 {
            assert_eq!(
                owner.prepare_with(candidate, |_| Err(RegistrationError::AllocationFailed)),
                Err(RegistrationError::AllocationFailed)
            );
        } else if phase == 3 {
            owner.prepare().unwrap();
        }
        drop(observer);
        let mask = LocalInterruptMask::new();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _table = MASTER_THREAD_TABLE.write();
            let _source = source.list.lock();
            drop(owner);
            assert_eq!(account.used(), phase + 1);
        }
        drop(mask);
    }
    // Explicitly close only this empty standalone fixture source. None of the
    // abandoned entries was linked, and no target exit is fabricated.
    source.notify_exit();
    assert!(source.source().is_none());
    assert!(original.is_closed());
    assert_eq!(original.registered(), 0);
    drop((original, source));
    let successor = watch_budget::DomainBudget::new(4);
    let replacement = ExitSource::new();
    assert_eq!(successor.used(), 0);
    assert_eq!(replacement.registered(), 0);
    drop((successor, replacement));
    assert_eq!(account.used(), 4);
    crate::logln!(
        "[exit watch owner retention] four guarded captured/source/candidate/entry phases \
         retained four original watch charges, three lists, one entry and four weak-only callback \
         allocations after empty fixture-source closure; no target thread or root retained"
    );
}

pub(super) fn run() {
    let irq = get_int_state();
    let watches = watch_budget::node_used();
    let lists = list_budget::node_used();
    rejection_and_unused();
    publication();
    assert_eq!(watch_budget::node_used(), watches);
    assert_eq!(list_budget::node_used(), lists);
    guarded_abandonment();
    assert_eq!(watch_budget::node_used(), (watches.0 + 4, watches.1 + 4));
    assert_eq!(list_budget::node_used(), (lists.0 + 3, lists.1 + 3));
    assert_eq!(get_int_state(), irq);
    crate::logln!(
        "[exit watch owner] retained guarded list/borrowed-entry rejection, four ordered unused \
         finishes, final-weak disposal, 64 heap/table-held publications and post-guard finishes \
         with actual token cancellation passed"
    );
}
