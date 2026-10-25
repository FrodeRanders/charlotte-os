//! Serialized pre-AP record phases use private original accounts, no live root.

use super::*;
use crate::{
    cpu::{
        isa::lp::ops::get_int_state,
        multiprocessor::interrupt_tracking::LocalInterruptMask,
    },
    klib::observer::{
        CallOnNotify,
        Observer,
        WaitSponsor,
        list_budget,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn context() {
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(COMPLETIONS.try_write().is_some());
}

fn owner(account: &Arc<budget::DomainBudget>, id: OperationId) -> Preparing {
    Preparing::new(id, Some(alloc::vec![0x42; 16]), budget::reserve(account, false).unwrap())
}

fn admit(preparing: &mut Preparing) {
    preparing.admit_with(Admission::try_new_retaining).unwrap();
}

fn waiters(preparing: &mut Preparing) {
    preparing
        .waiters_with(|platform| ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform))
        .unwrap();
}

fn success_and_rejection() {
    let account = budget::DomainBudget::new(64);
    let ids = operation_ids::OperationIds::new(0);
    let mut records = Vec::with_capacity(64);
    for _ in 0..64 {
        let mut preparing = owner(&account, ids.allocate().unwrap());
        preparing
            .admit_with(|charge| {
                context();
                assert!(charge.is_some());
                Admission::try_new_retaining(charge)
            })
            .unwrap();
        preparing
            .waiters_with(|platform| {
                context();
                ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
            })
            .unwrap();
        let operation = preparing.0.operation;
        let pointer = preparing.0.buffer.as_ref().unwrap().as_ptr();
        let record = preparing.complete().unwrap();
        assert_eq!(record.operation_id(), operation);
        let inner = record.inner.lock();
        let buffer = inner.buffer.as_ref().unwrap();
        assert_eq!(buffer.as_ptr(), pointer, "record completion replaced its original buffer");
        assert_eq!(buffer.as_slice(), &[0x42; 16]);
        drop(inner);
        records.push(record);
    }
    let weak = Arc::downgrade(&records[0]);
    assert_eq!(account.used(), 64);
    context();
    drop(records);
    assert!(weak.upgrade().is_none());
    assert_eq!(account.used(), 1);
    let successor = budget::DomainBudget::new(1);
    let fresh = budget::reserve(&successor, false).unwrap();
    drop(weak);
    assert_eq!(account.used(), 0);
    assert_eq!(successor.used(), 1);
    drop(fresh);

    for reject_holder in [true, false] {
        let result = new_with(
            ids.allocate().unwrap(),
            Some(alloc::vec![0x43; 16]),
            budget::reserve(&account, false).unwrap(),
            |charge| {
                context();
                assert_eq!(account.used(), 1);
                if reject_holder {
                    Err(core::alloc::AllocError)
                } else {
                    Admission::try_new_retaining(charge)
                }
            },
            |_| {
                context();
                assert!(!reject_holder, "rejected holder reached list allocation");
                assert_eq!(account.used(), 1);
                Err(RegistrationError::AllocationFailed)
            },
        );
        assert!(matches!(result, Err(SubmitError::WouldBlock)));
        assert_eq!(account.used(), 0);
    }

    // Consume this private allocator's sole allowance to reject actual record
    // backing admission, without exhausting the global physical allocator.
    let mut preparing = owner(&account, ids.allocate().unwrap());
    admit(&mut preparing);
    let occupied = preparing.0.allocator.as_ref().unwrap().clone().try_arc(17u8).unwrap();
    drop(occupied);
    waiters(&mut preparing);
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("failed record notified"));
    let token =
        sponsor.register(preparing.0.waiters.as_ref().unwrap(), Arc::downgrade(&observer)).unwrap();
    assert_eq!(sponsor.used(), 1);
    assert!(matches!(preparing.complete(), Err(SubmitError::WouldBlock)));
    assert_eq!(account.used(), 0);
    assert_eq!(sponsor.used(), 0, "failed record did not close its source entries");
    assert_eq!(Arc::weak_count(&observer), 0);
    drop(token);
}

fn retained_rejection_and_cancel() {
    let account = budget::DomainBudget::new(1);
    for phase in 0..3 {
        let mut preparing = owner(&account, 1);
        let pointer = preparing.0.buffer.as_ref().unwrap().as_ptr();
        if phase >= 1 {
            admit(&mut preparing);
        }
        if phase >= 2 {
            waiters(&mut preparing);
        }
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _registry = COMPLETIONS.write();
            if phase == 0 {
                assert_eq!(
                    preparing.admit_with(|_| Err(core::alloc::AllocError)),
                    Err(SubmitError::WouldBlock)
                );
                assert!(preparing.0.charge.is_some());
            } else if phase == 1 {
                assert_eq!(
                    preparing.waiters_with(|_| Err(RegistrationError::AllocationFailed)),
                    Err(SubmitError::WouldBlock)
                );
                assert!(preparing.0.allocator.is_some());
            }
            assert_eq!(preparing.0.buffer.as_ref().unwrap().as_ptr(), pointer);
            assert_eq!(preparing.0.buffer.as_ref().unwrap().as_slice(), &[0x42; 16]);
            assert_eq!(account.used(), 1);
        }
        let mut checks = 0;
        preparing.cancel_with(|| {
            context();
            assert_eq!(account.used(), 1, "record admission refunded before payload disposal");
            checks += 1;
        });
        assert_eq!(checks, 3);
        assert_eq!(account.used(), 0);
    }
}

#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    let account = budget::DomainBudget::new(3);
    let lists_before = list_budget::node_used();
    for phase in 0..3 {
        let mut preparing = owner(&account, 1);
        if phase >= 1 {
            admit(&mut preparing);
        }
        if phase >= 2 {
            waiters(&mut preparing);
        }
        let mask = LocalInterruptMask::new();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _registry = COMPLETIONS.write();
            drop(preparing);
            assert_eq!(account.used(), phase + 1);
        }
        drop(mask);
    }
    assert_eq!(list_budget::node_used(), (lists_before.0 + 1, lists_before.1 + 1));
    crate::logln!(
        "[completion record construction retention] guarded buffer/charge, holder and list phases \
         retained three original record charges, three 16-byte buffers, two holders and one empty \
         source list; no root or authority"
    );
}

pub(super) fn run() {
    let irq = get_int_state();
    let baseline = budget::node_used();
    let lists = list_budget::node_used();
    success_and_rejection();
    retained_rejection_and_cancel();
    assert_eq!(budget::node_used(), baseline);
    assert_eq!(list_budget::node_used(), lists);
    guarded_abandonment();
    assert_eq!(budget::node_used(), (baseline.0 + 3, baseline.1 + 3));
    assert_eq!(get_int_state(), irq);
    crate::logln!(
        "[completion record construction] 64 exact buffer/operation transfers, weak-only original \
         admission, holder/list/backing rejection and payload-first explicit cancellation passed"
    );
}
