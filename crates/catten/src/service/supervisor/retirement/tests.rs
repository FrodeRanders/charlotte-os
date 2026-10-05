//! Serialized real-root probes; failures retain roots without recovery bypass.

use super::*;
use crate::{
    memory::{
        PHYSICAL_FRAME_ALLOCATOR,
        backing_budget::{
            self,
            Kind,
        },
        operation::{
            AddressSpaceOperation,
            OperationError,
        },
    },
    service::{
        loader,
        supervisor,
    },
};

pub(crate) fn run() {
    test_pending_success();
    test_timeout_registry();
    test_stale_owner();
    crate::logln!(
        "[supervisor retirement] pending ownership, close fencing, exact reuse, cached terminal \
         error and deployment claim/counter retention passed"
    );
}

fn domain() -> ServiceDomain {
    let handle = loader::create_user_address_space_handle();
    assert!(memory::commit_user_heap_page_handle(handle, charlotte_launch::HEAP_VADDR));
    // No ELF or running threads in these fixtures. The owner never accesses
    // bootstrap/status frames; deployment tests supply an already-pending
    // owner and a cached acknowledgement rather than reading dummy addresses.
    ServiceDomain {
        asid: handle.id(),
        address_space: handle,
        tid: 0,
        generation: 0,
        config_frame: 0u64.into(),
        status_frame: 0u64.into(),
    }
}

fn test_pending_success() {
    let before = backing_budget::test_used_pages(Kind::Heap);
    let domain = domain();
    let first = AddressSpaceOperation::acquire(domain.address_space).unwrap();
    let second = AddressSpaceOperation::acquire(domain.address_space).unwrap();
    let mut owner = DomainTeardown::new(domain);
    assert_eq!(owner.poll(), Ok(false));
    assert!(owner.closing.is_some());
    assert!(matches!(
        AddressSpaceOperation::acquire(domain.address_space),
        Err(OperationError::Closing)
    ));
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
    first.release().unwrap();
    assert_eq!(owner.poll(), Ok(false));
    second.release().unwrap();
    assert_eq!(owner.poll(), Ok(true));
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before);
    let fresh = loader::create_user_address_space_handle();
    assert_eq!(fresh.id(), domain.asid);
    assert_ne!(fresh, domain.address_space);
    // Completion is cached: polling the old owner cannot close a replacement.
    assert_eq!(owner.poll(), Ok(true));
    assert_eq!(memory::current_address_space_handle(fresh.id()), Some(fresh));
    memory::close_user_address_space_handle(fresh).unwrap();
}

fn test_timeout_registry() {
    const PRINCIPAL: u64 = 0x7465_6172_646f_776e;
    let before = backing_budget::test_used_pages(Kind::Heap);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let domain = domain();
    let retained = free - PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let operation = AddressSpaceOperation::acquire(domain.address_space).unwrap();
    let owner = DomainTeardown::with_deadline(domain, 0);
    let counter =
        supervisor::DEPLOYMENT_ACKNOWLEDGED_RETIREMENTS.load(core::sync::atomic::Ordering::Relaxed);
    let registry = crate::cpu::multiprocessor::spin::mutex::Mutex::new(alloc::vec::Vec::new());
    registry.lock().push(supervisor::DeployedDomain {
        principal: PRINCIPAL,
        domain,
        shutdown_grace_ms: 0,
        retirement_deadline_ms: Some(0),
        retirement_reason: charlotte_launch::lifecycle::REASON_DEPLOYMENT_RETIRED,
        force_requested: false,
        retirement_acknowledged: true,
        teardown: DeploymentTeardown::Polling,
    });
    let poll = || {
        crate::syscall::retire_deployed_artifact_with_registry(
            &registry,
            PRINCIPAL,
            false,
            charlotte_launch::lifecycle::REASON_DEPLOYMENT_RETIRED,
            0,
        )
    };
    assert_eq!(poll(), 1, "competing caller stole deployment close authority");
    {
        let mut entries = registry.lock();
        let entry = entries.iter_mut().find(|entry| entry.principal == PRINCIPAL).unwrap();
        entry.teardown = DeploymentTeardown::Pending(owner);
    }
    assert_eq!(poll(), u64::MAX);
    operation.release().unwrap();
    assert_eq!(poll(), u64::MAX, "terminal timeout was retried as successful retirement");
    assert_eq!(
        supervisor::DEPLOYMENT_ACKNOWLEDGED_RETIREMENTS.load(core::sync::atomic::Ordering::Relaxed),
        counter
    );
    {
        let entries = registry.lock();
        let entry = entries.iter().find(|entry| entry.principal == PRINCIPAL).unwrap();
        assert!(matches!(
            entry.teardown,
            DeploymentTeardown::Failed(DomainTeardownError::AddressSpace(
                AddressSpaceCloseError::OperationDrainTimedOut
            ))
        ));
    }
    assert!(matches!(
        AddressSpaceOperation::acquire(domain.address_space),
        Err(OperationError::Closing)
    ));
    assert_eq!(memory::current_address_space_handle(domain.asid), Some(domain.address_space));
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - retained);
    crate::service::shutdown::tests::test_failed_reclamation(domain);
    crate::logln!(
        "[supervisor retirement] timeout retains deployment entry/root retained_frames={} \
         heap_pages=1",
        retained
    );
    // Dropping isolated fixture bookkeeping does not refund the real closing
    // root, slot/counts or budgets. No recovery or live-registry mutation.
}

fn test_stale_owner() {
    let domain = domain();
    memory::close_user_address_space_handle(domain.address_space).unwrap();
    let fresh = loader::create_user_address_space_handle();
    assert_eq!(fresh.id(), domain.asid);
    let mut owner = DomainTeardown::new(domain);
    assert_eq!(
        owner.poll(),
        Err(DomainTeardownError::AddressSpace(AddressSpaceCloseError::StaleHandle))
    );
    assert_eq!(
        owner.poll(),
        Err(DomainTeardownError::AddressSpace(AddressSpaceCloseError::StaleHandle))
    );
    assert_eq!(memory::current_address_space_handle(fresh.id()), Some(fresh));
    memory::close_user_address_space_handle(fresh).unwrap();
}
