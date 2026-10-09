//! Complete private grant retention, not a hardware completion fixture.
use crate::{
    device::{
        PreparedDmaDomain,
        dma,
        dma_tables,
    },
    memory::{
        self,
        AddressSpaceCloseError,
        PHYSICAL_FRAME_ALLOCATOR,
    },
};

fn prepare() -> (memory::AddressSpaceHandle, PreparedDmaDomain) {
    let root = crate::service::loader::create_user_address_space_handle();
    let mut grant =
        PreparedDmaDomain::new(root.id(), |_| panic!("private owner reached hardware destruction"))
            .unwrap();
    let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
    grant.resources.admission.resources.reservation = Some(
        crate::capability::reserve_in_lifecycle(
            root.id(),
            crate::capability::ObjectKind::Device,
            &lifecycle,
        )
        .unwrap(),
    );
    grant.resources.creation.retain_private(dma::test_private_domain());
    drop(lifecycle);
    (root, grant)
}

fn retained(root: memory::AddressSpaceHandle) {
    assert_eq!(memory::current_address_space_handle(root.id()), Some(root));
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 1);
    let fresh = crate::service::loader::create_user_address_space_handle();
    assert_ne!(fresh.id(), root.id());
    memory::close_user_address_space_handle(fresh).unwrap();
}

#[allow(clippy::result_large_err)] // Fixture returns complete inline failure owners.
pub(super) fn run() {
    // Known-private completion never calls the registered-domain destroy adapter.
    let baseline = dma_tables::used();
    let (root, grant) = prepare();
    assert!(grant.resources.creation.private.is_some());
    assert!(grant.cancel_unpublished().is_ok());
    assert_eq!(dma_tables::used(), baseline);
    assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 0);
    memory::close_user_address_space_handle(root).unwrap();

    let baseline = dma_tables::used();
    let (root, mut grant) = prepare();
    let pages = grant.resources.creation.private.as_mut().unwrap().tables().pages();
    assert!(pages >= 2);
    assert_eq!(
        dma::create_domain_with_reset(
            u32::MAX,
            None,
            &mut grant.resources.creation,
            |_, _| panic!("armed private creation reached reset")
        ),
        Err(dma::Error::OperationInFlight)
    );
    crate::device::admission_tests::drop_grant_under_guards(grant);
    retained(root);
    assert_eq!(dma_tables::used().1, baseline.1 + pages);

    let (failed_root, grant) = prepare();
    let before_failed_release = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let mut calls = 0;
    let grant = grant
        .cancel_unpublished_with(|creation| {
            creation.cancel_private_with(|domain| {
                domain.cancel_with(|tables| {
                    tables.cancel_private_with(|frame| {
                        calls += 1;
                        if calls == 2 {
                            return Err(memory::physical::Error::InvalidPAddr);
                        }
                        PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame)
                    })
                })
            })
        })
        .err()
        .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(grant.resources.creation.id, None);
    assert!(grant.resources.creation.private.is_some());
    // The containing grant's terminal phase rejects before any second adapter.
    let mut grant = grant
        .cancel_unpublished_with(|_| panic!("private grant retried physical release"))
        .err()
        .unwrap();
    // The private table owner independently freezes before the first release.
    let domain = grant.resources.creation.private.take().unwrap();
    let domain = domain
        .cancel_with(|tables| {
            tables.cancel_private_with(|_| panic!("frozen private ledger touched returned frame"))
        })
        .err()
        .unwrap();
    grant.resources.creation.private = Some(domain);
    crate::device::admission_tests::drop_grant_under_guards(grant);
    retained(failed_root);
    assert_eq!(dma_tables::used().1, baseline.1 + 2 * pages);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before_failed_release + 1);
    crate::logln!(
        "[DMA private retention] complete grant Drop under guards, armed rejection and partial \
         release with no retry passed; {} original domain charges, {} frames, both exact roots \
         and authority reservations retained",
        2 * pages,
        2 * pages - 1
    );
}
