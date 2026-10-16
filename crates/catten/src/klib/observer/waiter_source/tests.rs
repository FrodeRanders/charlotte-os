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
    let rejected = source.register_with(Arc::downgrade(&observer), &sponsor, |_| {
        assert!(source.list.lock().is_none(), "allocation retains initialization guard");
        Err(RegistrationError::AllocationFailed)
    });
    assert_eq!(rejected.unwrap_err(), RegistrationError::AllocationFailed);
    assert!(source.list.lock().is_none());
    assert_eq!(list_budget::node_used(), baseline);
    let mut ordinary = Preparing::capture(&sponsor, Arc::downgrade(&observer));
    ordinary
        .prepare_list(&source, |platform| {
            assert!(source.list.lock().is_none());
            ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
        })
        .unwrap();
    sponsor.mark_platform();
    let mut platform = Preparing::capture(&sponsor, Arc::downgrade(&observer));
    platform
        .prepare_list(&source, |classification| {
            assert!(source.list.lock().is_none());
            ObserverList::try_new(waiter_budget::SOURCE_LIMIT, classification)
        })
        .unwrap();
    assert_eq!(list_budget::node_used(), (baseline.0 + 2, baseline.1 + 1));
    // A drain before first publication is only a hint, not stored readiness.
    assert!(source.drain().is_empty());
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        platform.select(&source);
        ordinary.select(&source);
        assert!(core::ptr::eq(&**platform.selected(), &**ordinary.selected()));
        assert!(core::ptr::eq(&**platform.selected(), &**platform.candidate()));
        assert!(!core::ptr::eq(&**platform.selected(), &**ordinary.candidate()));
    }
    // The losing allocation and its original ordinary charge survived all
    // slot/heap holds. Only explicit post-guard disposal refunds it.
    ordinary.release_candidate();
    platform.release_candidate();
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
    ordinary.finish();
    platform.finish();
    drop(source);
    assert_eq!(list_budget::node_used(), baseline);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[waiter source preparation] unlocked allocation/failure, competing initialization, \
         heap-held publication and post-guard original-charge disposal passed"
    );
}
