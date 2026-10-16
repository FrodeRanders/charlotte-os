//! Serialized containing-owner controls before/after callback entry construction.

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

const CLIENT: usize = 0x5e3c;

fn context(completion: &CompletionRef) {
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(COMPLETIONS.try_write().is_some());
    assert!(completion.inner.try_lock().is_some());
}

fn capture(
    completion: &CompletionRef,
    cap: CompletionCap,
    observer: &Arc<dyn Observer>,
) -> Preparing {
    Preparing::capture(Captured::new(CLIENT, cap, completion).unwrap(), Arc::downgrade(observer))
}

fn candidate(preparing: &mut Preparing) {
    preparing
        .candidate_with(|platform| {
            ObserverList::try_new(watch_budget::MAX_COMPLETION_CALLBACKS, platform)
        })
        .unwrap();
}

fn ordinary_rejection() {
    for mode in 0..3 {
        open_address_space(CLIENT, 4);
        let cap = submit(CLIENT, OpCode::Read, Some(alloc::vec![1; 16])).unwrap();
        let completion = completion_of(CLIENT, cap).unwrap();
        let account = watch_admission(CLIENT).unwrap();
        let observer: Arc<dyn Observer> =
            CallOnNotify::new(|| panic!("rejected callback notified"));
        let mut pressure = Vec::new();
        if mode == 0 {
            while let Ok(charge) = watch_budget::reserve(&account, false) {
                pressure.push(charge);
            }
        }
        let before = account.used();
        let result = Preparing::new_with(
            Captured::new(CLIENT, cap, &completion).unwrap(),
            Arc::downgrade(&observer),
            |platform| {
                assert_ne!(mode, 0, "quota rejection reached list allocation");
                if mode == 1 {
                    let _heap = PRIMARY_ALLOCATOR.lock();
                    let _registry = COMPLETIONS.write();
                    let _inner = completion.inner.lock();
                    assert_eq!(account.used(), 1);
                    assert_eq!(Arc::weak_count(&observer), 1);
                    Err(RegistrationError::AllocationFailed)
                } else {
                    context(&completion);
                    ObserverList::try_new(watch_budget::MAX_COMPLETION_CALLBACKS, platform)
                }
            },
            |preparing| {
                assert_eq!(mode, 2, "list rejection reached entry preparation");
                let _heap = PRIMARY_ALLOCATOR.lock();
                let _registry = COMPLETIONS.write();
                let _inner = completion.inner.lock();
                assert!(preparing.0.observer.is_some() && preparing.0.charge.is_some());
                assert!(preparing.0.candidate.is_some() && preparing.0.entry.is_none());
                assert_eq!(account.used(), 1);
                Err(ObserveError::ResourceLimit)
            },
        );
        assert!(matches!(result, Err(ObserveError::ResourceLimit)));
        assert_eq!(account.used(), before);
        assert_eq!(Arc::weak_count(&observer), 0);
        assert!(completion.inner.lock().callbacks.is_none());
        assert!(completion.holds_buffer());
        assert_eq!(completion.state_kind(), OpStateKind::InFlight);
        drop(pressure);
        // This serialized fixture never exposes the submitted operation.
        abort_submission(CLIENT, cap).unwrap();
        drop(completion);
        close_address_space(CLIENT);
        crate::capability::close_address_space(CLIENT);
    }
}

fn explicit_unused_and_retained_rejection() {
    open_address_space(CLIENT, 8);
    let cap = submit(CLIENT, OpCode::Read, Some(alloc::vec![2; 16])).unwrap();
    let completion = completion_of(CLIENT, cap).unwrap();
    let account = watch_admission(CLIENT).unwrap();
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("unused callback notified"));
    for phase in 0..4 {
        let mut preparing = capture(&completion, cap, &observer);
        if phase >= 1 {
            preparing.admit().unwrap();
        }
        if phase >= 2 {
            candidate(&mut preparing);
        }
        if phase >= 3 {
            preparing.entry().unwrap();
        }
        if phase == 1 {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _registry = COMPLETIONS.write();
            let _inner = completion.inner.lock();
            assert_eq!(
                preparing.candidate_with(|_| Err(RegistrationError::AllocationFailed)),
                Err(ObserveError::ResourceLimit)
            );
            assert!(preparing.0.observer.is_some() && preparing.0.charge.is_some());
            assert_eq!(account.used(), 1);
        }
        let mut step = 0;
        preparing.finish_with(|| {
            context(&completion);
            let still_charged = phase >= 1
                && step
                    < if phase == 3 {
                        3
                    } else {
                        4
                    };
            assert_eq!(account.used(), usize::from(still_charged));
            let weak_retained = step == 0 || (phase == 3 && step < 3);
            assert_eq!(Arc::weak_count(&observer), usize::from(weak_retained));
            step += 1;
        });
        assert_eq!(step, 5);
        assert_eq!(account.used(), 0);
    }
    // Discard the unpublished fixture submission without fabricating completion.
    abort_submission(CLIENT, cap).unwrap();
    drop(completion);
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
}

fn borrowed_publication() {
    open_address_space(CLIENT, 128);
    let cap = submit(CLIENT, OpCode::Read, Some(alloc::vec![3; 16])).unwrap();
    let completion = completion_of(CLIENT, cap).unwrap();
    let account = watch_admission(CLIENT).unwrap();
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("cancelled callback notified"));
    let mut preparations = Vec::with_capacity(64);
    let mut registrations = Vec::with_capacity(64);
    for _ in 0..64 {
        preparations.push(
            Preparing::new_with(
                Captured::new(CLIENT, cap, &completion).unwrap(),
                Arc::downgrade(&observer),
                |platform| {
                    context(&completion);
                    ObserverList::try_new(watch_budget::MAX_COMPLETION_CALLBACKS, platform)
                },
                |preparing| {
                    context(&completion);
                    assert!(preparing.0.observer.is_some() && preparing.0.charge.is_some());
                    preparing.entry()
                },
            )
            .unwrap(),
        );
    }
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for preparing in &mut preparations {
            registrations.push(preparing.publish().unwrap().unwrap());
        }
    }
    assert_eq!(account.used(), 64);
    for preparing in preparations {
        preparing.finish_with(|| {
            context(&completion);
            assert_eq!(account.used(), 64, "published entry lost its registration owner");
        });
    }
    assert_eq!(completion.inner.lock().callbacks.as_ref().unwrap().registered(), 64);
    drop(registrations);
    assert_eq!(account.used(), 0);
    assert_eq!(Arc::weak_count(&observer), 0);
    assert!(completion.holds_buffer());
    assert_eq!(completion.state_kind(), OpStateKind::InFlight);
    // Registration cancellation leaves this unpublished operation in flight.
    abort_submission(CLIENT, cap).unwrap();
    drop(completion);
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
}

#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    open_address_space(CLIENT, 8);
    let cap = submit(CLIENT, OpCode::Read, Some(alloc::vec![4; 16])).unwrap();
    let completion = completion_of(CLIENT, cap).unwrap();
    let weak = Arc::downgrade(&completion);
    let records = record_admission(CLIENT).unwrap();
    let watches = watch_admission(CLIENT).unwrap();
    for phase in 0..4 {
        let observer: Arc<dyn Observer> =
            CallOnNotify::new(|| panic!("abandoned callback notified"));
        let mut preparing = capture(&completion, cap, &observer);
        if phase >= 1 {
            preparing.admit().unwrap();
        }
        if phase >= 2 {
            candidate(&mut preparing);
        }
        if phase >= 3 {
            preparing.entry().unwrap();
        }
        drop(observer);
        let mask = LocalInterruptMask::new();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _registry = COMPLETIONS.write();
            let _inner = completion.inner.lock();
            drop(preparing);
            assert_eq!(records.used(), 1);
            assert_eq!(watches.used(), phase);
        }
        drop(mask);
    }
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    drop(completion);
    assert_eq!((records.used(), watches.used()), (1, 3));
    let retained = weak.upgrade().expect("abandoned owner lost exact captured completion");
    assert_eq!(retained.inner.lock().buffer.as_deref(), Some(&[4; 16][..]));
    assert!(retained.inner.lock().callbacks.is_none());
    drop((retained, weak));
    open_address_space(CLIENT, 8);
    assert_eq!(record_admission(CLIENT).unwrap().used(), 0);
    assert_eq!(watch_admission(CLIENT).unwrap().used(), 0);
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    assert_eq!((records.used(), watches.used()), (1, 3));
    crate::logln!(
        "[completion callback owner retention] guarded captured/charged/list/entry phases \
         retained one exact buffered record, three original watch charges, two candidate lists, \
         one entry and four weak-only callback allocations across namespace replacement"
    );
}

pub(super) fn run() {
    let irq = get_int_state();
    let records = budget::node_used();
    let watches = watch_budget::node_used();
    let lists = list_budget::node_used();
    ordinary_rejection();
    explicit_unused_and_retained_rejection();
    borrowed_publication();
    assert_eq!(budget::node_used(), records);
    assert_eq!(watch_budget::node_used(), watches);
    assert_eq!(list_budget::node_used(), lists);
    guarded_abandonment();
    assert_eq!(budget::node_used(), (records.0 + 1, records.1 + 1));
    assert_eq!(watch_budget::node_used(), (watches.0 + 3, watches.1 + 3));
    assert_eq!(list_budget::node_used(), (lists.0 + 3, lists.1 + 3));
    assert_eq!(get_int_state(), irq);
    crate::logln!(
        "[completion callback owner] quota/list/borrowed-entry rejection, four explicit unused \
         finishes, retained guarded rejection, 64 heap-held publications and post-guard finishes \
         with original charges and no fabricated callback passed"
    );
}
