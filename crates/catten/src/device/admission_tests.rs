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

fn destroy_ok(id: u64) -> Result<(), dma::Error> {
    assert_eq!(id, u64::MAX);
    DESTROYS.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

fn destroy_failed(id: u64) -> Result<(), dma::Error> {
    destroy_ok(id)?;
    Err(dma::Error::Unsupported)
}

pub(crate) fn test_admission() {
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
            || panic!("over-quota backend creation"),
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
        grant_dma_domain_with_backend(owner.id(), || Err(DeviceError::DmaUnavailable), destroy_ok),
        Err(DeviceError::DmaUnavailable)
    );
    assert_eq!(used(owner.id()), TEST_NAMESPACE_LIMIT - 1);
    for destroy in [destroy_ok as fn(u64) -> Result<(), dma::Error>, destroy_failed] {
        // Whitebox retirement in the backend pauses the grant after physical
        // creation but before publication. Normal teardown is lifecycle-fenced.
        let transient = crate::service::loader::create_user_address_space_handle();
        let before = DESTROYS.load(Ordering::Relaxed);
        assert_eq!(
            grant_dma_domain_with_backend(
                transient.id(),
                || {
                    crate::capability::retire_address_space(transient.id());
                    Ok(u64::MAX)
                },
                destroy
            ),
            Err(DeviceError::NamespaceRetired)
        );
        assert_eq!(DESTROYS.load(Ordering::Relaxed), before + 1);
        assert_eq!(used(transient.id()), 0);
        assert!(!DEVICES.lock().contains_key(&transient.id()));
        crate::memory::close_user_address_space_handle(transient).unwrap();
    }
    crate::memory::budget::retire(owner);
    assert_eq!(grant_mmio(owner.id(), 0x0900_0000, 1), Err(DeviceError::NamespaceRetired));
    assert_eq!(grant_interrupt(owner.id(), intid), Err(DeviceError::NamespaceRetired));
    assert_eq!(
        grant_dma_domain_with_backend(
            owner.id(),
            || panic!("retired backend creation"),
            destroy_ok
        ),
        Err(DeviceError::NamespaceRetired)
    );
    crate::memory::close_user_address_space_handle(owner).unwrap();
    test_reused_grant();
    retirement::tests::run();
    logln!(
        "[device admission] quota, MMIO/IRQ recovery, DMA create/refund/rollback and exact \
         namespace reuse passed"
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
