//! Kernel ABI fixtures intentionally retain raw IDs to inspect admission.
//! Fake DMA backends exercise creation/rollback ordering, not IOMMU hardware.

use super::*;
use crate::capability::admission_tests::{
    TEST_NAMESPACE_LIMIT,
    test_fill_remaining_namespace as fill,
    test_free_fixture_slot as free,
    test_namespace_used as used,
};

static DESTROYS: AtomicU32 = AtomicU32::new(0);
static EXPECTED_IRQ: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn destroy_ok(id: u64) -> Result<(), dma::Error> {
    assert_eq!(id, u64::MAX);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), EXPECTED_IRQ.load(Ordering::Relaxed));
    dma::test_assert_backend_available();
    drop(crate::memory::ADDRESS_SPACE_LIFECYCLE.try_lock().expect("rollback holds lifecycle"));
    drop(DEVICES.try_lock().expect("rollback holds device registry"));
    drop(crate::memory::ADDRESS_SPACE_TABLE.try_lock().expect("rollback holds root table"));
    drop(crate::memory::PHYSICAL_FRAME_ALLOCATOR.try_lock().expect("rollback holds allocator"));
    drop(
        crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR
            .try_lock()
            .expect("rollback holds heap"),
    );
    DESTROYS.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

fn destroy_failed(id: u64) -> Result<(), dma::Error> {
    destroy_ok(id)?;
    Err(dma::Error::Unsupported)
}

pub(crate) fn test_admission() {
    dma_tables::test_admission();
    mapping::test_admission();
    domain_creation::test_admission();
    crate::device_management::drivers::busses::pci_express::topology::reset::test_admission();
    let owner = crate::service::loader::create_user_address_space_handle();
    let intid = 225;
    assert!(!DEVICES.lock().values().any(|caps| {
        caps.caps
            .values()
            .any(|object| matches!(object, DeviceObject::Interrupt(irq) if irq.intid == intid))
    }));
    fill(owner.id());
    assert_eq!(grant_mmio(owner.id(), 0x0900_0000, 1), Err(DeviceError::ResourceLimit));
    assert_eq!(grant_interrupt(owner.id(), intid), Err(DeviceError::ResourceLimit));
    assert_eq!(interrupt_route_owner(intid), None);
    // Even an invalid requester cannot reach backend lookup/creation at quota.
    assert_eq!(grant_dma_domain(owner.id(), u32::MAX, None), Err(DeviceError::ResourceLimit));
    assert_eq!(
        grant_dma_domain_with_backend(
            owner.id(),
            |_| panic!("over-quota backend creation"),
            destroy_ok
        ),
        Err(DeviceError::ResourceLimit)
    );
    assert_eq!(used(owner.id()), TEST_NAMESPACE_LIMIT);
    assert!(!DEVICES.lock().contains_key(&owner.id()));
    free(owner.id());
    let cap = grant_mmio(owner.id(), 0x0900_0000, 1).unwrap();
    assert_eq!(used(owner.id()), TEST_NAMESPACE_LIMIT);
    mmio_map_any(owner.id(), cap, true).unwrap();
    mmio_unmap(owner.id(), cap).unwrap();
    close_cap(owner.id(), cap).unwrap();
    let irq = grant_interrupt(owner.id(), intid).unwrap();
    close_cap(owner.id(), irq).unwrap();
    assert_eq!(used(owner.id()), TEST_NAMESPACE_LIMIT - 1);
    assert_eq!(
        grant_dma_domain_with_backend(owner.id(), |_| Err(DeviceError::DmaUnavailable), destroy_ok),
        Err(DeviceError::DmaUnavailable)
    );
    assert_eq!(used(owner.id()), TEST_NAMESPACE_LIMIT - 1);
    for fails in [false, true] {
        let destroy = if fails {
            destroy_failed
        } else {
            destroy_ok
        };
        // Whitebox retirement in the backend pauses the grant after physical
        // creation but before publication. The exact root lease now also permits staged close
        // before publication.
        let transient = crate::service::loader::create_user_address_space_handle();
        let before = DESTROYS.load(Ordering::Relaxed);
        EXPECTED_IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Relaxed);
        assert_eq!(
            grant_dma_domain_with_backend(
                transient.id(),
                |creation| {
                    creation.record(u64::MAX);
                    crate::capability::retire_address_space(transient.id());
                    Ok(())
                },
                destroy
            ),
            Err(DeviceError::NamespaceRetired)
        );
        assert_eq!(DESTROYS.load(Ordering::Relaxed), before + 1);
        assert_eq!(used(transient.id()), usize::from(fails));
        assert!(!DEVICES.lock().contains_key(&transient.id()));
        if fails {
            assert_retained_root(transient);
        } else {
            crate::memory::close_user_address_space_handle(transient).unwrap();
        }
    }
    test_grant_abandonment();
    super::private_domain::test_admission();
    test_backend_creation_rejection();
    crate::memory::budget::retire(owner);
    assert_eq!(grant_mmio(owner.id(), 0x0900_0000, 1), Err(DeviceError::NamespaceRetired));
    assert_eq!(grant_interrupt(owner.id(), intid), Err(DeviceError::NamespaceRetired));
    assert_eq!(
        grant_dma_domain_with_backend(
            owner.id(),
            |_| panic!("retired backend creation"),
            destroy_ok
        ),
        Err(DeviceError::NamespaceRetired)
    );
    crate::memory::close_user_address_space_handle(owner).unwrap();
    test_reused_grant();
    retirement::tests::run();
    crate::memory::object::namespace_close::tests::run();
    logln!(
        "[device admission] quota, MMIO/IRQ recovery, DMA create/refund/rollback and exact \
         namespace reuse passed"
    );
}

fn assert_retained_root(handle: crate::memory::AddressSpaceHandle) {
    assert_eq!(crate::memory::current_address_space_handle(handle.id()), Some(handle));
    assert_eq!(
        crate::memory::close_user_address_space_handle(handle),
        Err(crate::memory::AddressSpaceCloseError::OperationsInFlight)
    );
    let fresh = crate::service::loader::create_user_address_space_handle();
    assert_ne!(fresh.id(), handle.id());
    let cap = grant_mmio(fresh.id(), 0x0900_0000, 1).unwrap();
    close_cap(fresh.id(), cap).unwrap();
    crate::memory::close_user_address_space_handle(fresh).unwrap();
    assert_eq!(used(handle.id()), 1);
}

fn prepare_fixture(
    has_domain: bool,
    fails: bool,
) -> (crate::memory::AddressSpaceHandle, PreparedDmaDomain) {
    let root = crate::service::loader::create_user_address_space_handle();
    let mut prepared = PreparedDmaDomain::new(
        root.id(),
        if fails {
            destroy_failed
        } else {
            destroy_ok
        },
    )
    .unwrap();
    let lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
    prepared.resources.reservation = Some(
        crate::capability::reserve_in_lifecycle(
            root.id(),
            crate::capability::ObjectKind::Device,
            &lifecycle,
        )
        .unwrap(),
    );
    if has_domain {
        prepared.resources.creation.record(u64::MAX);
    }
    drop(lifecycle);
    (root, prepared)
}

#[allow(clippy::drop_non_drop)] // Exercise abandonment of ManuallyDrop fields.
pub(super) fn drop_grant_under_guards(prepared: PreparedDmaDomain) {
    let before = DESTROYS.load(Ordering::Relaxed);
    let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
    dma::test_with_backend_locked(|| {
        let _devices = DEVICES.lock();
        let _table = crate::memory::ADDRESS_SPACE_TABLE.lock();
        let _kernel = crate::memory::KERNEL_AS.lock();
        crate::capability::admission_tests::test_with_registry_locked(|| {
            let physical = crate::memory::PHYSICAL_FRAME_ALLOCATOR.lock();
            let _heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
            let free = physical.free_frames();
            drop(prepared);
            assert_eq!(physical.free_frames(), free);
            assert_eq!(DESTROYS.load(Ordering::Relaxed), before);
        });
    });
}

fn test_grant_abandonment() {
    // Fake backend obligations: these do not install a hardware domain. The
    // real root and capability admission exercise implicit field destruction.
    for has_domain in [false, true] {
        let (root, mut prepared) = prepare_fixture(has_domain, false);
        assert_eq!(used(root.id()), 1);
        if has_domain {
            assert_eq!(
                dma::create_domain_with_reset(
                    u32::MAX,
                    None,
                    &mut prepared.resources.creation,
                    |_, _| panic!("armed creation obligation reached reset"),
                ),
                Err(dma::Error::OperationInFlight)
            );
            assert_eq!(prepared.resources.creation.id, Some(u64::MAX));
            assert_eq!(used(root.id()), 1);
        }
        drop_grant_under_guards(prepared);
        assert_retained_root(root);
    }
    let (root, prepared) = prepare_fixture(true, true);
    EXPECTED_IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Relaxed);
    let before = DESTROYS.load(Ordering::Relaxed);
    let prepared = prepared.cancel_unpublished().err().unwrap();
    assert_eq!(DESTROYS.load(Ordering::Relaxed), before + 1);
    let prepared = prepared.cancel_unpublished().err().unwrap();
    assert_eq!(DESTROYS.load(Ordering::Relaxed), before + 1);
    drop_grant_under_guards(prepared);
    assert_retained_root(root);
    // Explicit cancellation of an ordinary unused reservation refunds and
    // completes the lease. An unrelated successor can close normally.
    let (root, prepared) = prepare_fixture(false, false);
    assert!(prepared.cancel_unpublished().is_ok());
    assert_eq!(used(root.id()), 0);
    crate::memory::close_user_address_space_handle(root).unwrap();
    logln!(
        "[DMA grant rollback] unlocked one-shot rollback, guarded containing-owner Drop, exact \
         root/capability retention and successor isolation passed; four roots/reservations \
         retained"
    );
}

fn test_backend_creation_rejection() {
    for fails in [false, true] {
        let root = crate::service::loader::create_user_address_space_handle();
        let before = DESTROYS.load(Ordering::Relaxed);
        EXPECTED_IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Relaxed);
        assert_eq!(
            grant_dma_domain_with_backend(
                root.id(),
                |creation| {
                    // Unlike an ordinary pre-publication allocation rejection,
                    // this fake backend has installed its registered obligation
                    // before returning an error. The adapter must not lose it.
                    creation.record(u64::MAX);
                    Err(DeviceError::DmaUnavailable)
                },
                if fails {
                    destroy_failed
                } else {
                    destroy_ok
                },
            ),
            Err(DeviceError::DmaUnavailable)
        );
        assert_eq!(DESTROYS.load(Ordering::Relaxed), before + 1);
        assert!(!DEVICES.lock().contains_key(&root.id()));
        assert_eq!(used(root.id()), usize::from(fails));
        if fails {
            assert_retained_root(root);
        } else {
            crate::memory::close_user_address_space_handle(root).unwrap();
        }
    }
    logln!(
        "[DMA creation ownership] backend error preserves registered obligation through unlocked \
         rollback; confirmed refund and failed exact root/reservation retention passed (one \
         additional retained root/reservation)"
    );
}

fn test_reused_grant() {
    let old = crate::service::loader::create_user_address_space_handle();
    let lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
    let staged = crate::capability::reserve_in_lifecycle(
        old.id(),
        crate::capability::ObjectKind::Device,
        &lifecycle,
    )
    .unwrap();
    let id = staged.identity();
    drop(lifecycle);
    assert_eq!(mmio_map_any(old.id(), id, true), Err(DeviceError::UnknownCapability));
    crate::memory::close_user_address_space_handle(old).unwrap();
    let fresh = crate::service::loader::create_user_address_space_handle();
    assert_eq!(fresh.id(), old.id());
    let cap = grant_mmio(fresh.id(), 0x0900_0000, 1).unwrap();
    assert_eq!(id, cap);
    assert_eq!(staged.publish(), Err(crate::capability::AllocationError::Retired));
    assert_eq!(used(fresh.id()), 1);
    mmio_map_any(fresh.id(), cap, false).unwrap();
    close_cap(fresh.id(), cap).unwrap();
    assert_eq!(used(fresh.id()), 0);
    crate::memory::close_user_address_space_handle(fresh).unwrap();
}

pub(crate) fn test_reset_grant_retention(
    source: crate::device_management::drivers::busses::pci_express::topology::reset::ResetSource<
        'static,
    >,
    uncertain: bool,
) {
    let (root, mut grant) = prepare_fixture(false, false);
    grant.resources.creation.reset = Some(source);
    if uncertain {
        grant = grant.cancel_unpublished().expect_err("uncertain reset refunded grant");
        assert!(grant.resources.creation.reset.is_some());
        grant = grant.cancel_unpublished_with(|_| panic!("uncertain reset retried")).err().unwrap();
    }
    drop_grant_under_guards(grant);
    assert_retained_root(root);
}
