//! Serialized kernel fixtures for list allocation admission.

use super::*;

/// Serialized boot fixture: exact counters and counter-only pool saturation.
pub(super) fn test_list_backing_admission() {
    use alloc::vec::Vec;

    use crate::{
        completion::{
            self,
            OpCode,
            OpResult,
            watch_budget,
        },
        klib::observer::{
            CallOnNotify,
            WaitSponsor,
            waiter_source::WaiterSource,
        },
    };
    let baseline = list_budget::node_used();
    let account = watch_budget::DomainBudget::new(1);
    let callback: Arc<dyn Observer> = CallOnNotify::new(|| panic!("discarded list notified"));
    let list = ObserverList::try_new(1, false).unwrap();
    let weak = Arc::downgrade(&list.0);
    let aliases: Vec<_> = (0..128).map(|_| weak.clone()).collect();
    let token = list
        .register(Arc::downgrade(&callback), watch_budget::reserve(&account, false).unwrap())
        .unwrap();
    let batch = list.close();
    drop(batch);
    assert_eq!(account.used(), 0, "entry admission is independent of retained list backing");
    drop(list);
    assert!(weak.upgrade().is_some(), "empty token retains the source allocation");
    assert_eq!(list_budget::node_used(), (baseline.0 + 1, baseline.1 + 1));
    drop(token);
    assert!(weak.upgrade().is_none());
    assert_eq!(list_budget::node_used(), (baseline.0 + 1, baseline.1 + 1));
    drop(aliases);
    assert_eq!(list_budget::node_used(), (baseline.0 + 1, baseline.1 + 1));
    drop(weak);
    assert_eq!(list_budget::node_used(), baseline);
    assert_eq!(
        ObserverList::<watch_budget::Charge>::try_new_with(1, false, |_, _| Err(
            core::alloc::AllocError
        ))
        .unwrap_err(),
        RegistrationError::AllocationFailed
    );
    assert_eq!(list_budget::node_used(), baseline);

    // First admission captures classification. Promotion does not reclassify
    // an existing source, and a retired waiter cannot initialize another list.
    let sponsor = WaitSponsor::new(false);
    let first = WaiterSource::new();
    drop(first.register(Arc::downgrade(&callback), &sponsor).unwrap());
    sponsor.mark_platform();
    let second = WaiterSource::new();
    drop(second.register(Arc::downgrade(&callback), &sponsor).unwrap());
    assert_eq!(list_budget::node_used(), (baseline.0 + 2, baseline.1 + 1));
    sponsor.retire();
    let retired = WaiterSource::new();
    assert_eq!(
        retired.register(Arc::downgrade(&callback), &sponsor).unwrap_err(),
        RegistrationError::Closed
    );
    assert_eq!(list_budget::node_used(), (baseline.0 + 2, baseline.1 + 1));
    drop((first, second, retired));
    assert_eq!(list_budget::node_used(), baseline);

    // Real subsystem rejection paths. Scalar capabilities are confined to
    // this kernel ABI fixture; no application resource owner is duplicated.
    let root = crate::service::loader::create_user_address_space_handle();
    let asid = root.id();
    completion::open_address_space(asid, 4);
    completion::open_cq(asid, 0, 4).unwrap();
    let endpoint = crate::ipc::endpoint_create(asid, 1, 1, 1).unwrap();
    let connection =
        crate::ipc::connection_mint(asid, endpoint, crate::ipc::ConnectionRights::ALL).unwrap();
    let cap = completion::submit(asid, OpCode::Read, Some(alloc::vec![7])).unwrap();
    let records = completion::record_admission(asid).unwrap();
    let watches = completion::watch_admission(asid).unwrap();
    let endpoint_account = crate::ipc::endpoint_admission(asid).unwrap();
    let before_endpoint = endpoint_account.used();
    let ordinary = WaitSponsor::new(false);
    let blocked_source = WaiterSource::new();
    let mut charges = Vec::new();
    while let Ok(charge) = list_budget::reserve(false) {
        charges.push(charge);
    }
    assert_eq!(list_budget::node_used().1, list_budget::ORDINARY_LIMIT);
    assert_eq!(
        ObserverList::<watch_budget::Charge>::try_new(1, false).unwrap_err(),
        RegistrationError::ResourceLimit
    );
    assert_eq!(
        blocked_source.register(Arc::downgrade(&callback), &ordinary).unwrap_err(),
        RegistrationError::ResourceLimit
    );
    assert_eq!(ordinary.used(), 0);
    assert_eq!(
        completion::submit(asid, OpCode::Nop, None),
        Err(completion::SubmitError::WouldBlock)
    );
    assert_eq!(records.used(), 1);
    assert_eq!(completion::open_cq(asid, 1, 4), Err(completion::CqOpenError::AllocationFailed));
    assert_eq!(
        completion::observe(asid, cap, callback.clone()).unwrap_err(),
        completion::ObserveError::ResourceLimit
    );
    assert_eq!(watches.used(), 0);
    assert!(completion::holds_buffer(asid, cap).unwrap());
    assert_eq!(
        crate::ipc::endpoint_create(asid, 2, 1, 1),
        Err(crate::ipc::IpcError::ResourceLimit)
    );
    assert_eq!(endpoint_account.used(), before_endpoint);
    assert_eq!(
        crate::ipc::scalar_call(asid, connection, 1, 2),
        Err(crate::ipc::IpcError::ResourceLimit)
    );
    assert_eq!(crate::ipc::endpoint_status(asid, endpoint).unwrap().1, 0);
    let before_platform = list_budget::node_used();
    let platform = WaitSponsor::new(true);
    let progress = WaiterSource::new();
    drop(progress.register(Arc::downgrade(&callback), &platform).unwrap());
    assert_eq!(list_budget::node_used(), (before_platform.0 + 1, before_platform.1));
    drop(progress);
    assert_eq!(list_budget::node_used(), before_platform);
    while let Ok(charge) = list_budget::reserve(true) {
        charges.push(charge);
    }
    assert_eq!(list_budget::node_used(), (list_budget::NODE_LIMIT, list_budget::ORDINARY_LIMIT));
    assert_eq!(
        ObserverList::<watch_budget::Charge>::try_new(1, true).unwrap_err(),
        RegistrationError::ResourceLimit
    );
    drop(charges);
    // Recovery follows actual release: the failed lazy source can initialize.
    drop(blocked_source.register(Arc::downgrade(&callback), &ordinary).unwrap());
    drop(blocked_source);
    let recovered = completion::submit(asid, OpCode::Nop, None).unwrap();
    completion::abort_submission(asid, recovered).unwrap();
    completion::open_cq(asid, 1, 4).unwrap();
    completion::complete(asid, cap, OpResult::Ok(0)).unwrap();
    completion::close(asid, cap).unwrap();
    crate::ipc::close_cap(asid, connection).unwrap();
    crate::ipc::close_cap(asid, endpoint).unwrap();
    crate::ipc::close_address_space_fixture(asid).unwrap();
    completion::close_address_space(asid);
    crate::memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(list_budget::node_used(), baseline);
    crate::logln!(
        "[observer lists] retained token/weak backing, rollback, captured classification, node \
         reserve and subsystem rejection passed"
    );
}
