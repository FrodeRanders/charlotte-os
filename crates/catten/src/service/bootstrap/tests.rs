//! Real image-backed roots; no dummy page is ever accessed after rejection.
use core::sync::atomic::{
    AtomicU32,
    Ordering,
};

use super::*;
use crate::{
    memory::{
        self,
        AddressSpaceCloseError,
        PHYSICAL_FRAME_ALLOCATOR,
        backing_budget::{
            self,
            Kind,
        },
        retirement::{
            CloseProgress,
            ClosingAddressSpace,
        },
    },
    service::{
        loader,
        supervisor,
    },
};

pub(crate) fn run() {
    test_success_and_staged_close();
    test_wrong_backing_and_force_failure();
    test_reused_root();
    test_abandoned_access();
    crate::logln!(
        "[service page ownership] exact backing, close fencing, reuse rejection, cached shutdown \
         failures and abandoned image charges passed"
    );
}

fn fixture() -> supervisor::ServiceDomain {
    loader::admission_tests::service_pages_fixture()
}

fn state(pages: &ServicePages<'_>) -> u32 {
    let base: *const u8 = pages.config.into();
    unsafe { &*base.add(charlotte_launch::lifecycle::CONTROL_STATE_OFFSET).cast::<AtomicU32>() }
        .load(Ordering::Acquire)
}

fn acknowledge(pages: &ServicePages<'_>, value: u32) {
    let base: *const u8 = pages.status.into();
    unsafe { &*base.add(charlotte_launch::lifecycle::STATUS_STATE_OFFSET).cast::<AtomicU32>() }
        .store(value, Ordering::Release);
}

fn test_success_and_staged_close() {
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let image = backing_budget::test_used_pages(Kind::Image);
    let domain = fixture();
    with_service_pages(&domain, |pages| {
        assert!(memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
        assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
        assert_eq!(
            memory::close_user_address_space_handle(domain.address_space),
            Err(AddressSpaceCloseError::OperationsInFlight)
        );
        pages.write_request(
            charlotte_launch::lifecycle::STATE_DRAIN_REQUESTED,
            charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
            123,
        );
        assert_eq!(state(pages), charlotte_launch::lifecycle::STATE_DRAIN_REQUESTED);
        acknowledge(pages, charlotte_launch::lifecycle::STATUS_READY);
        assert_eq!(pages.lifecycle_status(), charlotte_launch::lifecycle::STATUS_READY);
    })
    .unwrap();
    let root = AddressSpaceOperation::acquire(domain.address_space).unwrap();
    let closing = ClosingAddressSpace::begin(domain.address_space).unwrap();
    assert_eq!(
        with_service_pages(&domain, |_| panic!("closing root accessed pages")),
        Err::<(), _>(ServicePageError::Root(OperationError::Closing))
    );
    // An older admitted access borrows its existing root, including force
    // publication; it does not reopen admission after staged close.
    let pages = ServicePages::borrow(&root, &domain).unwrap();
    assert_eq!(pages.lifecycle_status(), charlotte_launch::lifecycle::STATUS_READY);
    pages.write_request(
        charlotte_launch::lifecycle::STATE_FORCE_TERMINATING,
        charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
        123,
    );
    let closing = match closing.poll().unwrap() {
        CloseProgress::Pending(owner) => owner,
        _ => panic!("close consumed live page access"),
    };
    root.release().unwrap();
    assert!(matches!(closing.poll().unwrap(), CloseProgress::Complete));
    assert_eq!(backing_budget::test_used_pages(Kind::Image), image);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
}

fn test_wrong_backing_and_force_failure() {
    let domain = fixture();
    for wrong in [
        supervisor::ServiceDomain {
            config_frame: domain.status_frame,
            ..domain
        },
        supervisor::ServiceDomain {
            status_frame: 0u64.into(),
            ..domain
        },
        supervisor::ServiceDomain {
            asid: domain.asid + 1,
            ..domain
        },
    ] {
        assert_eq!(
            with_service_pages(&wrong, |_| panic!("foreign backing accessed")),
            Err::<(), _>(ServicePageError::WrongBacking)
        );
    }
    let foreign = fixture();
    let root = AddressSpaceOperation::acquire(foreign.address_space).unwrap();
    assert!(matches!(ServicePages::borrow(&root, &domain), Err(ServicePageError::WrongBacking)));
    root.release().unwrap();
    memory::close_user_address_space_handle(foreign.address_space).unwrap();
    use crate::cpu::scheduler::system_scheduler::{
        self,
        Error,
    };
    assert!(matches!(
        system_scheduler::abort_domain_threads_with_request(domain.address_space, |root| {
            let wrong = supervisor::ServiceDomain {
                config_frame: 0u64.into(),
                ..domain
            };
            ServicePages::borrow(root, &wrong).map_err(|_| Error::ThreadTerminated)?;
            panic!("rejected force request published");
        }),
        Err(Error::ThreadTerminated)
    ));
    // Rejected publication released its lease but kept the terminal thread fence.
    assert!(ADDRESS_SPACE_TABLE.lock().get(domain.asid).unwrap().thread_admission_closed);
    with_service_pages(&domain, |pages| {
        assert_eq!(state(pages), charlotte_launch::lifecycle::STATE_RUNNING);
    })
    .unwrap();
    memory::close_user_address_space_handle(domain.address_space).unwrap();
}

fn test_reused_root() {
    let old = fixture();
    memory::close_user_address_space_handle(old.address_space).unwrap();
    let fresh = fixture();
    assert_eq!(fresh.asid, old.asid);
    assert_ne!(fresh.address_space, old.address_space);
    // Even matching successor physical addresses cannot qualify a stale handle.
    let stale = supervisor::ServiceDomain {
        config_frame: fresh.config_frame,
        status_frame: fresh.status_frame,
        ..old
    };
    assert_eq!(
        with_service_pages(&stale, |_| panic!("successor page accessed")),
        Err::<(), _>(ServicePageError::Root(OperationError::StaleHandle))
    );
    crate::service::shutdown::tests::test_rejected_service_pages(stale);
    with_service_pages(&fresh, |pages| {
        assert_eq!(state(pages), charlotte_launch::lifecycle::STATE_RUNNING);
        assert_eq!(pages.lifecycle_status(), 0);
    })
    .unwrap();
    memory::close_user_address_space_handle(fresh.address_space).unwrap();
}

fn test_abandoned_access() {
    let image = backing_budget::test_used_pages(Kind::Image);
    let domain = fixture();
    let root = AddressSpaceOperation::acquire(domain.address_space).unwrap();
    ServicePages::borrow(&root, &domain).unwrap();
    // Same root-owner abandonment as an unwinding/abandoned callback. No Drop
    // can refund pages or make the root reusable after unfinished access.
    drop(root);
    assert_eq!(
        memory::close_user_address_space_handle(domain.address_space),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(backing_budget::test_used_pages(Kind::Image), image + 2);
    with_service_pages(&domain, |pages| {
        assert_eq!(state(pages), charlotte_launch::lifecycle::STATE_RUNNING)
    })
    .unwrap();
    assert_eq!(memory::current_address_space_handle(domain.asid), Some(domain.address_space));
}
