//! Kernel-only grants/claims are inspected with raw fixture IDs.

use super::*;
use crate::capability::{
    self,
    AllocationError,
    ObjectKind,
    admission_tests::*,
};

pub(crate) fn test_admission() {
    let live_observer = *SYSTEM_OBSERVER_ASID.lock();
    if live_observer.is_some() {
        assert!(matches!(
            ObserverLaunchClaim::acquire(&OBSERVER_LAUNCH_CLAIMED),
            Err(ObserverLaunchError::AlreadyStarted)
        ));
    }
    let flag = AtomicBool::new(false);
    let claim = ObserverLaunchClaim::acquire(&flag).unwrap();
    assert!(matches!(
        ObserverLaunchClaim::acquire(&flag),
        Err(ObserverLaunchError::AlreadyStarted)
    ));
    drop(claim);
    drop(ObserverLaunchClaim::acquire(&flag).unwrap());
    let handle = loader::create_user_address_space_handle();
    test_fill_remaining_namespace(handle.id());
    assert_eq!(grant_system_observer(handle), Err(AllocationError::ResourceLimit));
    assert_eq!(test_namespace_used(handle.id()), TEST_NAMESPACE_LIMIT);
    test_free_fixture_slot(handle.id());
    let cap = grant_system_observer(handle).unwrap();
    assert_eq!(test_namespace_used(handle.id()), TEST_NAMESPACE_LIMIT);
    assert!(capability::contains(handle.id(), cap, ObjectKind::SystemObserver));
    assert!(capability::remove(handle.id(), cap, ObjectKind::SystemObserver));
    crate::memory::budget::retire(handle);
    assert_eq!(grant_system_observer(handle), Err(AllocationError::Retired));
    close_user_address_space_handle(handle).unwrap();
    let fresh = loader::create_user_address_space_handle();
    assert_eq!(fresh.id(), handle.id());
    assert_ne!(fresh, handle);
    let cap = grant_system_observer(fresh).unwrap();
    assert_eq!(grant_system_observer(handle), Err(AllocationError::Retired));
    assert!(capability::contains(fresh.id(), cap, ObjectKind::SystemObserver));
    assert_eq!(test_namespace_used(fresh.id()), 1);
    close_user_address_space_handle(fresh).unwrap();
    test_preparation_cleanup();
    assert_eq!(*SYSTEM_OBSERVER_ASID.lock(), live_observer);
    crate::logln!(
        "[observer admission] quota recovery, startup claim cancellation and exact generation \
         fencing passed"
    );
}

fn test_preparation_cleanup() {
    let live_observer = *SYSTEM_OBSERVER_ASID.lock();
    let flag = AtomicBool::new(false);
    let grantor = loader::create_user_address_space_handle();
    let endpoint = ipc::endpoint_create(grantor.id(), 1, 1, 4).unwrap();
    for leave_slot in [false, true] {
        let target = loader::create_user_address_space_handle();
        test_fill_remaining_namespace(target.id());
        if leave_slot {
            test_free_fixture_slot(target.id());
        }
        let records = crate::ipc::record_budget::node_used();
        let claim = ObserverLaunchClaim::acquire(&flag).unwrap();
        // Only the pre-bootstrap admission phase runs. The fixture has no ELF
        // frames; dummy addresses are never read/written or started here.
        let loaded = loader::LoadedDomain {
            asid: target.id(),
            address_space: target,
            entry_vaddr: 0,
            config_frame: PAddr::from(0u64),
            status_frame: PAddr::from(0u64),
        };
        let result = PreparingObserver::new(loaded, grantor.id(), endpoint);
        if leave_slot {
            assert!(matches!(result, Err(ObserverLaunchError::Admission)));
        } else {
            assert!(matches!(
                result,
                Err(ObserverLaunchError::BootstrapConnection(ipc::IpcError::ResourceLimit))
            ));
        }
        drop(result);
        drop(claim);
        assert_eq!(crate::memory::current_address_space_handle(target.id()), None);
        assert_eq!(crate::ipc::record_budget::node_used(), records);
        assert_eq!(test_namespace_used(grantor.id()), 1);
        assert_eq!(*SYSTEM_OBSERVER_ASID.lock(), live_observer);
        drop(ObserverLaunchClaim::acquire(&flag).unwrap());
    }
    close_user_address_space_handle(grantor).unwrap();
}
