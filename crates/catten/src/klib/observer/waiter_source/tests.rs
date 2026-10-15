//! Serialized admitted-list preparation, competing first publication and
//! post-guard losing-storage disposal through the production adapters.

use alloc::sync::Arc;

use super::*;
use crate::{
    klib::observer::{
        CallOnNotify,
        list_budget,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let baseline = list_budget::node_used();
    let source = WaiterSource::new();
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("unused source notified"));
    let rejected = source.prepare_list(&sponsor, |_| {
        assert!(source.list.lock().is_none(), "allocation retains initialization guard");
        Err(RegistrationError::AllocationFailed)
    });
    assert_eq!(rejected.unwrap_err(), RegistrationError::AllocationFailed);
    assert!(source.list.lock().is_none());
    assert_eq!(list_budget::node_used(), baseline);
    let ordinary = source
        .prepare_list(&sponsor, |platform| {
            assert!(source.list.lock().is_none());
            ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
        })
        .unwrap();
    sponsor.mark_platform();
    let platform = source
        .prepare_list(&sponsor, |classification| {
            assert!(source.list.lock().is_none());
            ObserverList::try_new(waiter_budget::SOURCE_LIMIT, classification)
        })
        .unwrap();
    assert_eq!(list_budget::node_used(), (baseline.0 + 2, baseline.1 + 1));
    // A drain before first publication is only a hint, not stored readiness.
    assert!(source.drain().is_empty());
    let winner;
    let loser_result;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        winner = source.publish_list(&platform);
        loser_result = source.publish_list(&ordinary);
        assert!(core::ptr::eq(&*winner, &*loser_result));
        assert!(core::ptr::eq(&*winner, &*platform));
        assert!(!core::ptr::eq(&*winner, &*ordinary));
    }
    // The losing allocation and its original ordinary charge survived all
    // slot/heap holds. Only explicit post-guard disposal refunds it.
    drop(ordinary);
    assert_eq!(list_budget::node_used(), (baseline.0 + 1, baseline.1));
    let token = source.register(Arc::downgrade(&observer), &sponsor).unwrap();
    assert_eq!(sponsor.used(), 1);
    drop(token);
    assert_eq!(sponsor.used(), 0);
    sponsor.retire();
    assert_eq!(
        source.register(Arc::downgrade(&observer), &sponsor).unwrap_err(),
        RegistrationError::Closed
    );
    let empty = WaiterSource::new();
    assert_eq!(
        empty.register(Arc::downgrade(&observer), &sponsor).unwrap_err(),
        RegistrationError::Closed
    );
    assert!(empty.list.lock().is_none());
    drop((source, winner, loser_result, platform));
    assert_eq!(list_budget::node_used(), baseline);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[waiter source preparation] unlocked allocation/failure, competing initialization, \
         heap-held publication and post-guard original-charge disposal passed"
    );
}
