//! Serialized pre-AP entry phases with actual list/heap guard holds.

use alloc::vec::Vec;

use super::*;
use crate::{
    completion::watch_budget as budget,
    cpu::{
        isa::lp::ops::get_int_state,
        multiprocessor::interrupt_tracking::LocalInterruptMask,
    },
    klib::observer::CallOnNotify,
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn context(list: &ListRef<budget::Charge>) {
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(list.state.try_lock().is_some());
}

fn allocate<C>() -> Result<Box<MaybeUninit<EntryNode<C>>>, RegistrationError> {
    Box::try_new_uninit().map_err(|_| RegistrationError::AllocationFailed)
}

fn completed_entries() {
    let account = budget::DomainBudget::new(64);
    let list = ObserverList::<budget::Charge>::try_new(64, false).unwrap();
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("private entry notified"));
    let identity = Arc::as_ptr(&observer) as *const ();
    let mut preparing = Vec::with_capacity(64);
    let mut completed = Vec::with_capacity(64);
    let mut tokens = Vec::with_capacity(32);
    for _ in 0..64 {
        let mut owner = PreparingEntry::new(
            Arc::downgrade(&observer),
            budget::reserve(&account, false).unwrap(),
        );
        owner.allocate(allocate).unwrap();
        preparing.push(owner);
    }
    let mask = LocalInterruptMask::new();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _list = list.state.lock();
        for owner in preparing.drain(..) {
            let entry = owner.complete();
            let node = &entry.0.as_ref().unwrap().node;
            assert_eq!(node.observer.as_ptr() as *const (), identity);
            assert_eq!(node.id, 0);
            assert!(node.next.is_none());
            completed.push(entry);
        }
        assert_eq!(account.used(), 64);
        assert_eq!(Arc::weak_count(&observer), 64);
    }
    // Completed unused preparation retains its existing active Drop policy.
    // This explicit fixture disposal is outside both local guards.
    context(&list);
    completed.truncate(32);
    assert_eq!(account.used(), 32);
    assert_eq!(Arc::weak_count(&observer), 32);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for entry in &mut completed {
            tokens.push(list.register_prepared(entry).unwrap());
            assert!(entry.0.is_none());
        }
    }
    assert_eq!(list.registered(), 32);
    context(&list);
    drop((completed, tokens));
    assert_eq!(list.registered(), 0);
    assert_eq!(account.used(), 0);
    assert_eq!(Arc::weak_count(&observer), 0);
    drop(mask);
}

fn rejected_and_unused() {
    let account = budget::DomainBudget::new(1);
    let list = ObserverList::<budget::Charge>::try_new(1, false).unwrap();
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("rejected entry notified"));
    for allocated in [false, true] {
        let mut owner = PreparingEntry::new(
            Arc::downgrade(&observer),
            budget::reserve(&account, false).unwrap(),
        );
        if allocated {
            owner.allocate(allocate).unwrap();
        } else {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _list = list.state.lock();
            assert_eq!(
                owner.allocate(|| Err(RegistrationError::AllocationFailed)),
                Err(RegistrationError::AllocationFailed)
            );
            assert_eq!(account.used(), 1);
            assert_eq!(Arc::weak_count(&observer), 1);
            assert!(owner.0.observer.is_some() && owner.0.charge.is_some());
        }
        let mut checks = 0;
        owner.cancel_with(|| {
            context(&list);
            assert_eq!(account.used(), 1);
            assert_eq!(Arc::weak_count(&observer), usize::from(checks < 2));
            checks += 1;
        });
        assert_eq!(checks, 3);
        assert_eq!(account.used(), 0);
    }
    // The ordinary constructor releases the final weak backing only after
    // this rejecting allocator callback and its actual guards have returned.
    let final_weak = Arc::downgrade(&observer);
    drop(observer);
    let result = prepare(final_weak, budget::reserve(&account, false).unwrap(), || {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _list = list.state.lock();
        assert_eq!(account.used(), 1);
        Err(RegistrationError::AllocationFailed)
    });
    assert!(matches!(result, Err(RegistrationError::AllocationFailed)));
    assert_eq!(account.used(), 0);
}

#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    let account = budget::DomainBudget::new(2);
    let list = ObserverList::<budget::Charge>::try_new(1, false).unwrap();
    for allocated in [false, true] {
        let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("abandoned entry notified"));
        let mut owner = PreparingEntry::new(
            Arc::downgrade(&observer),
            budget::reserve(&account, false).unwrap(),
        );
        if allocated {
            owner.allocate(allocate).unwrap();
        }
        // Each owner now holds that callback allocation's last weak reference.
        drop(observer);
        assert!(owner.0.observer.as_ref().unwrap().upgrade().is_none());
        let mask = LocalInterruptMask::new();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _list = list.state.lock();
            drop(owner);
            assert_eq!(account.used(), 1 + usize::from(allocated));
        }
        drop(mask);
    }
    drop(list);
    assert_eq!(account.used(), 2);
    crate::logln!(
        "[observer entry construction retention] guarded pre-storage and empty-storage phases \
         retained two original watch charges, two final weak callback allocations and one empty \
         entry Box; no list charge, token or user root retained"
    );
}

pub(super) fn run() {
    let irq = get_int_state();
    let before = budget::node_used();
    let lists = list_budget::node_used();
    completed_entries();
    rejected_and_unused();
    assert_eq!(budget::node_used(), before);
    assert_eq!(list_budget::node_used(), lists);
    guarded_abandonment();
    assert_eq!(budget::node_used(), (before.0 + 2, before.1 + 2));
    assert_eq!(list_budget::node_used(), lists);
    assert_eq!(get_int_state(), irq);
    crate::logln!(
        "[observer entry construction] 64 heap/list-held initializations, exact weak identity, 32 \
         unused disposals, 32 publications/cancellations, retaining rejection, final-weak \
         allocator-return disposal and explicit backing/weak-before-charge passed"
    );
}
